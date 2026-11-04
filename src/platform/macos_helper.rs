//! macOS TUN privilege boundary. The ordinary GUI talks to a pinned root helper;
//! neither the GUI nor an executable supplied over IPC is run as root.
use crate::kernel::KernelKind;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use uuid::Uuid;

const PROTOCOL: u32 = 1;
const MAX_FRAME: usize = 64 * 1024;
const MAX_CONFIG: u64 = 16 * 1024 * 1024;
const GEO_FILES: &[&str] = &["geoip.dat", "geosite.dat", "Country.mmdb"];
const IO_TIMEOUT: Duration = Duration::from_millis(500);
const SOL_LOCAL: libc::c_int = 0;
const LOCAL_PEERPID: libc::c_int = 2;
const CS_OPS_STATUS: u32 = 0;
const CS_OPS_CDHASH: u32 = 5;
const CS_VALID: u32 = 1;
const CS_RUNTIME: u32 = 0x10000;
static STOP_HELPER: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn csops(
        pid: libc::pid_t,
        ops: u32,
        useraddr: *mut libc::c_void,
        usersize: usize,
    ) -> libc::c_int;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Manifest {
    protocol: u32,
    gui_sha256: String,
    core_sha256: String,
    geodata: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Request {
    Hello {},
    Start {
        manifest: Manifest,
        config: PathBuf,
        log: PathBuf,
    },
    Status {
        session: String,
    },
    Stop {
        session: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "state", deny_unknown_fields)]
enum Response {
    Ready { manifest: Manifest },
    Running { session: String, pid: u32 },
    Exited { status: i32 },
    Error { message: String },
}

struct Paths {
    label: String,
    install: PathBuf,
    runtime: PathBuf,
    socket_dir: PathBuf,
    socket: PathBuf,
    plist: PathBuf,
}

impl Paths {
    fn new(uid: u32) -> Self {
        let label = format!("com.mateoki.vproxy.tun.{uid}");
        let socket_dir = PathBuf::from("/var/run").join(&label);
        Self {
            install: PathBuf::from("/Library/PrivilegedHelperTools").join(&label),
            runtime: PathBuf::from("/Library/Application Support/com.mateoki.vproxy.tun")
                .join(uid.to_string()),
            plist: PathBuf::from("/Library/LaunchDaemons").join(format!("{label}.plist")),
            socket: socket_dir.join("control.sock"),
            socket_dir,
            label,
        }
    }
}

struct Account {
    uid: u32,
    gid: u32,
    home: PathBuf,
    data_dirs: [PathBuf; 2],
}

fn account(uid: u32) -> Result<Account> {
    if uid == 0 {
        bail!("TUN helper refuses a root GUI account");
    }
    // getpwuid_r uses the account database, never HOME supplied by a caller.
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = std::ptr::null_mut();
    let mut buf = vec![0u8; 64 * 1024];
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut pwd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut found,
        )
    };
    if rc != 0 || found.is_null() {
        bail!("Cannot resolve macOS user {uid}");
    }
    let home = PathBuf::from(unsafe { CStr::from_ptr(pwd.pw_dir) }.to_str()?);
    require_clean_absolute(&home)?;
    // Match the two fixed macOS locations used by data_root_from(), including
    // installations that still retain their data under the old application name.
    let data_dirs = [
        home.join("Library/Application Support/vproxy"),
        home.join("Library/Application Support/vpn-parser"),
    ];
    Ok(Account {
        uid,
        gid: pwd.pw_gid,
        home,
        data_dirs,
    })
}

fn require_clean_absolute(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        bail!("Path must be absolute and contain no traversal components");
    }
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        bail!("NUL in path");
    }
    Ok(())
}

fn require_user_subpath(path: &Path, base: &Path) -> Result<()> {
    require_clean_absolute(path)?;
    if path == base || !path.starts_with(base) {
        bail!("Path is outside the VProxy data directory");
    }
    Ok(())
}

fn require_account_subpath(account: &Account, path: &Path) -> Result<()> {
    let base = account
        .data_dirs
        .iter()
        .find(|base| path.starts_with(base))
        .context("Path is outside the VProxy data directories")?;
    require_user_subpath(path, base)
}

/// Walk with openat(O_NOFOLLOW), retaining directory FDs. Renaming a user-owned
/// directory while this runs cannot redirect the final open into a root path.
fn open_user_file(account: &Account, path: &Path, append: bool) -> Result<File> {
    require_account_subpath(account, path)?;
    let mut directory = File::open("/")?;
    let mut walked = PathBuf::from("/");
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(p) => Some(p),
            _ => None,
        })
        .collect();
    for (index, part) in parts.iter().enumerate() {
        walked.push(part);
        let last = index + 1 == parts.len();
        let name = CString::new(part.as_encoded_bytes())?;
        let flags = if last {
            if append {
                libc::O_WRONLY | libc::O_APPEND
            } else {
                libc::O_RDONLY
            }
        } else {
            libc::O_RDONLY | libc::O_DIRECTORY
        };
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error())
                .with_context(|| format!("Open {} without symlinks", walked.display()));
        }
        let next = unsafe { File::from_raw_fd(fd) };
        let meta = next.metadata()?;
        if walked.starts_with(&account.home) {
            if meta.uid() != account.uid {
                bail!("{} is not owned by the GUI user", walked.display());
            }
        } else if meta.uid() != 0 || meta.mode() & 0o022 != 0 {
            bail!("Untrusted ancestor {}", walked.display());
        }
        if last && !meta.is_file() {
            bail!("Expected an existing regular file");
        }
        if !last && !meta.is_dir() {
            bail!("Expected a directory");
        }
        directory = next;
    }
    Ok(directory)
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn current_manifest(executable: &Path, data_root: &Path) -> Result<Manifest> {
    let mut geodata = BTreeMap::new();
    for name in GEO_FILES {
        let path = data_root.join(name);
        if path.exists() {
            geodata.insert((*name).into(), file_sha256(&path)?);
        }
    }
    Ok(Manifest {
        protocol: PROTOCOL,
        gui_sha256: file_sha256(&std::env::current_exe()?)?,
        core_sha256: file_sha256(executable)?,
        geodata,
    })
}

fn process_hash(pid: libc::pid_t) -> Result<[u8; 20]> {
    let mut flags: u32 = 0;
    let rc = unsafe {
        csops(
            pid,
            CS_OPS_STATUS,
            (&mut flags as *mut u32).cast(),
            std::mem::size_of_val(&flags),
        )
    };
    if rc != 0 || flags & (CS_VALID | CS_RUNTIME) != (CS_VALID | CS_RUNTIME) {
        bail!("VProxy must have a valid hardened-runtime code signature");
    }
    let mut hash = [0u8; 20];
    if unsafe { csops(pid, CS_OPS_CDHASH, hash.as_mut_ptr().cast(), hash.len()) } != 0 {
        return Err(io::Error::last_os_error()).context("Read peer code signature");
    }
    Ok(hash)
}

fn process_instance(pid: libc::pid_t) -> Result<(u64, u64)> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if read != size {
        return Err(io::Error::last_os_error()).context("Read parent process identity");
    }
    Ok((info.pbi_start_tvsec, info.pbi_start_tvusec))
}

fn authenticated_peer(
    stream: &UnixStream,
    expected_uid: u32,
    expected_hash: &[u8; 20],
) -> Result<libc::pid_t> {
    let (mut uid, mut gid) = (0, 0);
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error()).context("Read Unix peer credentials");
    }
    if uid != expected_uid {
        bail!("Unix peer UID mismatch");
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of_val(&pid) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            SOL_LOCAL,
            LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut len,
        )
    } != 0
        || pid <= 0
    {
        return Err(io::Error::last_os_error()).context("Read kernel-authenticated peer PID");
    }
    if process_hash(pid)? != *expected_hash {
        bail!("VProxy code signature changed; reinstall the helper");
    }
    Ok(pid)
}

fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_FRAME {
        bail!("IPC frame too large");
    }
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(&body)?;
    Ok(())
}

fn read_frame<T: for<'a> Deserialize<'a>>(stream: &mut UnixStream) -> Result<T> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME {
        bail!("Invalid IPC frame length");
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

fn rpc(paths: &Paths, request: &Request, timeout: Duration) -> Result<Response> {
    // A normal user cannot replace the root-owned parent directory or its socket.
    let parent = fs::symlink_metadata(&paths.socket_dir)?;
    if !parent.is_dir() || parent.uid() != 0 || parent.mode() & 0o022 != 0 {
        bail!("Untrusted helper socket directory");
    }
    let mut stream = UnixStream::connect(&paths.socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let hash = process_hash(unsafe { libc::getpid() })?;
    authenticated_peer(&stream, 0, &hash)?;
    write_frame(&mut stream, request)?;
    let response: Response = read_frame(&mut stream)?;
    match response {
        Response::Error { message } => Err(anyhow!(message)),
        response => Ok(response),
    }
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
fn apple_quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
fn path_quote(path: &Path) -> Result<String> {
    Ok(shell_quote(path.to_str().context("Non-UTF-8 path")?))
}
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn app_bundle(executable: &Path) -> Result<PathBuf> {
    let macos = executable.parent().context("No executable directory")?;
    let contents = macos.parent().context("No Contents directory")?;
    let bundle = contents.parent().context("No app bundle")?;
    if macos.file_name() != Some("MacOS".as_ref())
        || contents.file_name() != Some("Contents".as_ref())
        || bundle.extension() != Some("app".as_ref())
    {
        bail!("请从已签名的 VProxy.app 安装 TUN 服务");
    }
    Ok(bundle.to_path_buf())
}

fn helper_architecture() -> &'static str {
    // Rust calls the Apple Silicon architecture aarch64; arch(1) uses arm64.
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        std::env::consts::ARCH
    }
}

fn daemon_plist(paths: &Paths, uid: u32, gid: u32) -> String {
    let exe = paths.install.join("VProxyHelper.app/Contents/MacOS/vproxy");
    // A universal GUI may run under Rosetta. Pin launchd to the same slice so
    // mutual CDHash authentication compares the same signed executable slice.
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array><string>/usr/bin/arch</string><string>-{}</string><string>{}</string><string>--macos-tun-helper</string><string>{uid}</string><string>{gid}</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>AbandonProcessGroup</key><false/><key>ThrottleInterval</key><integer>5</integer><key>StandardOutPath</key><string>/dev/null</string><key>StandardErrorPath</key><string>/dev/null</string></dict></plist>\n", xml_escape(&paths.label), helper_architecture(), xml_escape(&exe.to_string_lossy()))
}

/// Read-only installation check, invoked as the GUI user by the authorized
/// installer. The protected manifest and authenticated Hello must both match
/// before the installer can discard the preceding installation.
pub fn check_installation(uid: u32) -> Result<()> {
    if unsafe { libc::getuid() } != uid || unsafe { libc::geteuid() } != uid {
        bail!("TUN installation check must run as the GUI user");
    }
    let _ = account(uid)?;
    let paths = Paths::new(uid);
    let actual_exe = std::env::current_exe()?.canonicalize()?;
    if actual_exe != paths.install.join("VProxyHelper.app/Contents/MacOS/vproxy") {
        bail!("TUN installation check must run from the installed helper");
    }
    let mut bytes = Vec::new();
    root_file(&paths.install.join("manifest.json"))?
        .take(MAX_FRAME as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FRAME {
        bail!("Installed manifest is too large");
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.protocol != PROTOCOL || file_sha256(&actual_exe)? != manifest.gui_sha256 {
        bail!("Installed helper manifest mismatch");
    }
    match rpc(&paths, &Request::Hello {}, IO_TIMEOUT)? {
        Response::Ready { manifest: active } if active == manifest => Ok(()),
        _ => bail!("TUN helper is not ready with the installed manifest"),
    }
}

fn install_transaction_prelude(
    paths: &Paths,
    stage: &Path,
    backup: &Path,
    plist_stage: &Path,
    plist_backup: &Path,
) -> Result<String> {
    Ok(format!(
        r#"set -eu
umask 077
vproxy_stage={stage}
vproxy_backup={backup}
vproxy_plist_stage={plist_stage}
vproxy_plist_backup={plist_backup}
vproxy_install={install}
vproxy_plist={plist}
vproxy_job={job}
vproxy_swapping=0
vproxy_new_install=0
vproxy_old_install=0
vproxy_new_plist=0
vproxy_old_plist=0
vproxy_committed=0
vproxy_cleanup() {{
  vproxy_result=$?
  trap - EXIT HUP INT TERM
  if test "$vproxy_committed" -eq 0 && test "$vproxy_swapping" -eq 1; then
    /bin/launchctl bootout "$vproxy_job" 2>/dev/null || true
    if test "$vproxy_new_install" -eq 1; then /bin/mv "$vproxy_install" "$vproxy_stage" || /bin/echo 'Could not move the failed TUN installation aside' >&2; fi
    if test "$vproxy_old_install" -eq 1; then /bin/mv "$vproxy_backup" "$vproxy_install" || /bin/echo "Could not restore the previous TUN installation; retained at $vproxy_backup" >&2; fi
    if test "$vproxy_new_plist" -eq 1; then /bin/rm -f "$vproxy_plist" || /bin/echo 'Could not remove the failed TUN service plist' >&2; fi
    if test "$vproxy_old_plist" -eq 1; then /bin/mv "$vproxy_plist_backup" "$vproxy_plist" || /bin/echo "Could not restore the previous TUN plist; retained at $vproxy_plist_backup" >&2; fi
    if test -f "$vproxy_plist"; then /bin/launchctl bootstrap system "$vproxy_plist" 2>/dev/null || /bin/echo 'Could not restart the previous TUN service' >&2; fi
  fi
  # Recursive cleanup is limited to this invocation's generated root staging path.
  /bin/rm -rf "$vproxy_stage" || true
  /bin/rm -f "$vproxy_plist_stage" || true
  exit "$vproxy_result"
}}
trap vproxy_cleanup EXIT
trap 'exit 1' HUP INT TERM
"#,
        stage = path_quote(stage)?,
        backup = path_quote(backup)?,
        plist_stage = path_quote(plist_stage)?,
        plist_backup = path_quote(plist_backup)?,
        install = path_quote(&paths.install)?,
        plist = path_quote(&paths.plist)?,
        job = shell_quote(&format!("system/{}", paths.label))
    ))
}

struct InstallScriptInputs<'a> {
    paths: &'a Paths,
    user: &'a Account,
    executable: &'a Path,
    data_root: &'a Path,
    manifest: &'a Manifest,
    bundle: &'a Path,
    stage: &'a Path,
    staging_root: &'a Path,
    backup: &'a Path,
    plist_staging: &'a Path,
    plist_backup: &'a Path,
}

// Pure command construction: no file access, authorization or system mutation.
fn installation_script(input: InstallScriptInputs<'_>) -> Result<String> {
    let InstallScriptInputs {
        paths,
        user,
        executable,
        data_root,
        manifest,
        bundle,
        stage,
        staging_root,
        backup,
        plist_staging,
        plist_backup,
    } = input;
    let manifest_path = stage.join("manifest.json");
    let plist_path = stage.join("helper.plist");
    let q = |p: &Path| path_quote(p);
    let mut script =
        install_transaction_prelude(paths, &staging_root, &backup, &plist_staging, &plist_backup)?;
    script.push_str(&format!("/bin/mkdir -p /Library/PrivilegedHelperTools /Library/LaunchDaemons\n/bin/mkdir {}\n/usr/bin/ditto --norsrc --noextattr --noacl {} {}\n/bin/cp {} {}\n/bin/mkdir {}\n", q(&staging_root)?, q(&bundle)?, q(&staging_root.join("VProxyHelper.app"))?, q(executable)?, q(&staging_root.join("mihomo"))?, q(&staging_root.join("geodata"))?));
    // This small app has no framework symlinks. A copied main-executable
    // symlink must never leave launchd following a user-writable target.
    script.push_str(&format!("if /usr/bin/find {} -type l -print -quit | /usr/bin/grep -q .; then /bin/echo 'TUN helper bundle must not contain symbolic links' >&2; exit 1; fi\n", q(&staging_root.join("VProxyHelper.app"))?));
    // Finder/File Provider metadata can be reattached while copying from
    // Documents. Clear it only on this generated, symlink-free snapshot;
    // the original app and its signed file contents are never modified.
    script.push_str(&format!(
        "/usr/bin/xattr -cr {}\n",
        q(&staging_root.join("VProxyHelper.app"))?
    ));
    // Checksums are literals embedded in the authorization request, so source
    // replacement during the password dialog cannot change installed code.
    let mut checks = vec![
        (
            staging_root.join("VProxyHelper.app/Contents/MacOS/vproxy"),
            manifest.gui_sha256.clone(),
        ),
        (staging_root.join("mihomo"), manifest.core_sha256.clone()),
    ];
    for (name, sha) in &manifest.geodata {
        if !GEO_FILES.contains(&name.as_str()) {
            bail!("Unknown geodata file");
        }
        script.push_str(&format!(
            "/bin/cp {} {}\n",
            q(&data_root.join(name))?,
            q(&staging_root.join("geodata").join(name))?
        ));
        checks.push((staging_root.join("geodata").join(name), sha.clone()));
    }
    for (file, sha) in checks {
        script.push_str(&format!(
            "test \"$(/usr/bin/shasum -a 256 {} | /usr/bin/cut -d ' ' -f 1)\" = {}\n",
            q(&file)?,
            shell_quote(&sha)
        ));
    }
    script.push_str(&format!(
        "/usr/bin/codesign --verify --deep --strict {}\n/bin/cp {} {}\n/bin/cp {} {}\n",
        q(&staging_root.join("VProxyHelper.app"))?,
        q(&manifest_path)?,
        q(&staging_root.join("manifest.json"))?,
        q(&plist_path)?,
        q(&plist_staging)?
    ));
    // The metadata files themselves must also be pinned before root trusts them.
    script.push_str(&format!("test \"$(/usr/bin/shasum -a 256 {} | /usr/bin/cut -d ' ' -f 1)\" = {}\ntest \"$(/usr/bin/shasum -a 256 {} | /usr/bin/cut -d ' ' -f 1)\" = {}\n", q(&staging_root.join("manifest.json"))?, shell_quote(&format!("{:x}", Sha256::digest(serde_json::to_vec(manifest)?))), q(&plist_staging)?, shell_quote(&format!("{:x}", Sha256::digest(daemon_plist(paths, user.uid, user.gid).as_bytes())))));
    // The manifest contains only file hashes and is readable by the ordinary-user installation checker.
    script.push_str(&format!(
        "/bin/chmod 644 {}\n",
        q(&staging_root.join("manifest.json"))?
    ));
    script.push_str(&format!("/usr/sbin/chown -R root:wheel {}\n/bin/chmod -R go-w {}\n/bin/chmod 755 {} {}\n/usr/sbin/chown root:wheel {}\n/bin/chmod 644 {}\n/usr/bin/plutil -lint {} >/dev/null\n", q(&staging_root)?, q(&staging_root)?, q(&staging_root)?, q(&staging_root.join("mihomo"))?, q(&plist_staging)?, q(&plist_staging)?, q(&plist_staging)?));
    // Keep both the app/core snapshot and plist until the new authenticated
    // service is ready. Any failure in the transaction restores the old pair.
    script.push_str(r#"vproxy_swapping=1
    /bin/launchctl bootout "$vproxy_job" 2>/dev/null || true
    if test -e "$vproxy_install"; then /bin/mv "$vproxy_install" "$vproxy_backup"; vproxy_old_install=1; fi
    if test -e "$vproxy_plist"; then /bin/mv "$vproxy_plist" "$vproxy_plist_backup"; vproxy_old_plist=1; fi
    /bin/mv "$vproxy_stage" "$vproxy_install"
    vproxy_new_install=1
    /bin/mv "$vproxy_plist_stage" "$vproxy_plist"
    vproxy_new_plist=1
    /bin/launchctl bootstrap system "$vproxy_plist"
    /bin/launchctl kickstart "$vproxy_job"
    vproxy_ready=0
    vproxy_attempt=0
    "#);
    let checker = paths.install.join("VProxyHelper.app/Contents/MacOS/vproxy");
    script.push_str(&format!("while test \"$vproxy_attempt\" -lt 40; do\n  if /usr/bin/sudo -u {} -- /usr/bin/arch -{} {} --macos-tun-helper-check {} >/dev/null 2>&1; then vproxy_ready=1; break; fi\n  vproxy_attempt=$((vproxy_attempt + 1))\n  /bin/sleep 0.1\ndone\n", shell_quote(&format!("#{}", user.uid)), helper_architecture(), q(&checker)?, user.uid));
    script.push_str(r#"if test "$vproxy_ready" -ne 1; then /bin/echo 'TUN helper did not pass the authenticated installation check' >&2; exit 1; fi
    vproxy_committed=1
    # Only this transaction's preceding snapshot is removed after successful Hello.
    /bin/rm -rf "$vproxy_backup" || /bin/echo "Could not remove the previous TUN installation at $vproxy_backup" >&2
    /bin/rm -f "$vproxy_plist_backup" || /bin/echo "Could not remove the previous TUN plist at $vproxy_plist_backup" >&2
    "#);
    Ok(script)
}

/// Installs only after the standard macOS administrator authorization dialog.
/// All elevated commands are fixed here, never read from a user-writable script.
fn install(
    paths: &Paths,
    user: &Account,
    executable: &Path,
    data_root: &Path,
    manifest: &Manifest,
) -> Result<()> {
    let app_exe = std::env::current_exe()?.canonicalize()?;
    let bundle = app_bundle(&app_exe)?;
    let stage = std::env::temp_dir().join(format!("vproxy-tun-install-{}", Uuid::new_v4()));
    fs::create_dir(&stage)?;
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))?;
    let result = (|| -> Result<()> {
        let manifest_path = stage.join("manifest.json");
        let plist_path = stage.join("helper.plist");
        fs::write(&manifest_path, serde_json::to_vec(manifest)?)?;
        fs::write(&plist_path, daemon_plist(paths, user.uid, user.gid))?;
        let staging_root = PathBuf::from("/Library/PrivilegedHelperTools").join(format!(
            ".{}.{}",
            paths.label,
            Uuid::new_v4()
        ));
        let backup = PathBuf::from("/Library/PrivilegedHelperTools").join(format!(
            ".{}.backup.{}",
            paths.label,
            Uuid::new_v4()
        ));
        let plist_staging = paths
            .plist
            .with_extension(format!("plist.{}", Uuid::new_v4()));
        let plist_backup = paths
            .plist
            .with_extension(format!("plist.backup.{}", Uuid::new_v4()));
        let script = installation_script(InstallScriptInputs {
            paths,
            user,
            executable,
            data_root,
            manifest,
            bundle: &bundle,
            stage: &stage,
            staging_root: &staging_root,
            backup: &backup,
            plist_staging: &plist_staging,
            plist_backup: &plist_backup,
        })?;
        let apple = format!(
            "do shell script {} with administrator privileges with prompt {}",
            apple_quote(&script),
            apple_quote("VProxy 需要安装或更新 TUN 后台服务，后续启用 TUN 无需重复输入密码。")
        );
        let output = Command::new("/usr/bin/osascript")
            .args(["-e", &apple])
            .output()?;
        if !output.status.success() {
            bail!(
                "TUN 后台服务安装未完成：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&stage); // Only this user-owned, random scratch directory.
    result
}

pub fn start(
    kernel: KernelKind,
    executable: &Path,
    data_root: &Path,
    config: &Path,
    log: &Path,
) -> Result<MacChild> {
    if kernel != KernelKind::Mihomo {
        bail!("macOS TUN 后台服务目前仅支持 Mihomo，请选择 Mihomo 内核");
    }
    let user = account(unsafe { libc::getuid() })?;
    let paths = Paths::new(user.uid);
    require_account_subpath(&user, executable)?;
    require_account_subpath(&user, config)?;
    require_account_subpath(&user, log)?;
    require_account_subpath(&user, data_root)?;
    // The log is always created by the GUI account. Root can only append after
    // checking the open descriptor; it never creates a user-supplied pathname.
    if !log.exists() {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(log)?;
    }
    let _ = open_user_file(&user, config, false)?;
    let _ = open_user_file(&user, log, true)?;
    let manifest = current_manifest(executable, data_root)?;
    let ready = matches!(rpc(&paths, &Request::Hello {}, IO_TIMEOUT), Ok(Response::Ready { manifest: installed }) if installed == manifest);
    if !ready {
        install(&paths, &user, executable, data_root, &manifest)?;
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if matches!(rpc(&paths, &Request::Hello {}, IO_TIMEOUT), Ok(Response::Ready { manifest: installed }) if installed == manifest)
            {
                break;
            }
            if Instant::now() >= deadline {
                bail!("TUN 后台服务安装后未就绪，请检查系统设置中的后台项目");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    match rpc(
        &paths,
        &Request::Start {
            manifest,
            config: config.into(),
            log: log.into(),
        },
        Duration::from_secs(3),
    )? {
        Response::Running { session, pid } => Ok(MacChild {
            paths,
            session,
            pid,
            exited: None,
        }),
        response => bail!("Unexpected TUN helper response: {response:?}"),
    }
}

pub struct MacChild {
    paths: Paths,
    session: String,
    pid: u32,
    exited: Option<ExitStatus>,
}

impl MacChild {
    pub fn id(&self) -> Option<u32> {
        self.exited.is_none().then_some(self.pid)
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.exited {
            return Ok(Some(status));
        }
        let response = rpc(
            &self.paths,
            &Request::Status {
                session: self.session.clone(),
            },
            IO_TIMEOUT,
        )
        .map_err(io::Error::other)?;
        match response {
            Response::Running { pid, .. } if pid == self.pid => Ok(None),
            Response::Exited { status } => {
                let status = ExitStatus::from_raw(status);
                self.exited = Some(status);
                Ok(Some(status))
            }
            _ => Err(io::Error::other("Unexpected helper child status")),
        }
    }
    pub async fn kill(&mut self) -> io::Result<()> {
        if self.exited.is_some() {
            return Ok(());
        }
        match rpc(
            &self.paths,
            &Request::Stop {
                session: self.session.clone(),
            },
            Duration::from_secs(3),
        )
        .map_err(io::Error::other)?
        {
            Response::Exited { status } => {
                self.exited = Some(ExitStatus::from_raw(status));
                Ok(())
            }
            _ => Err(io::Error::other("Helper did not reap the stopped kernel")),
        }
    }
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for MacChild {
    fn drop(&mut self) {
        if self.exited.is_none() {
            let _ = rpc(
                &self.paths,
                &Request::Stop {
                    session: self.session.clone(),
                },
                Duration::from_secs(3),
            );
        }
    }
}

struct Running {
    session: String,
    parent: libc::pid_t,
    parent_instance: (u64, u64),
    child: Child,
    status: Option<ExitStatus>,
}

impl Running {
    fn poll(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            self.status = self.child.try_wait()?;
        }
        Ok(self.status)
    }
    fn stop(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.poll()? {
            return Ok(status);
        }
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = self.poll()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                self.child.kill()?;
                let status = self.child.wait()?;
                self.status = Some(status);
                return Ok(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn secure_root_dir(path: &Path, mode: u32) -> Result<()> {
    // These paths are fixed by uid and never originate in IPC.
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
        bail!("Untrusted root directory {}", path.display());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn root_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
        bail!("Untrusted root-owned file {}", path.display());
    }
    Ok(file)
}

unsafe extern "C" fn stop_signal(_: libc::c_int) {
    STOP_HELPER.store(true, Ordering::Relaxed);
}

pub fn run_helper(uid: u32, gid: u32) -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("TUN helper must be launched by the system service");
    }
    let user = account(uid)?;
    if gid != user.gid {
        bail!("Helper account group mismatch");
    }
    let paths = Paths::new(uid);
    let actual_exe = std::env::current_exe()?.canonicalize()?;
    if actual_exe != paths.install.join("VProxyHelper.app/Contents/MacOS/vproxy") {
        bail!("Helper must run from its installed immutable path");
    }
    secure_root_dir(&paths.install, 0o755)?;
    let own_hash = process_hash(unsafe { libc::getpid() })?;
    let mut manifest_bytes = Vec::new();
    root_file(&paths.install.join("manifest.json"))?
        .take(MAX_FRAME as u64)
        .read_to_end(&mut manifest_bytes)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.protocol != PROTOCOL
        || file_sha256(&actual_exe)? != manifest.gui_sha256
        || file_sha256(&paths.install.join("mihomo"))? != manifest.core_sha256
    {
        bail!("Installed helper manifest mismatch");
    }
    let _ = root_file(&paths.install.join("mihomo"))?;
    secure_root_dir(
        Path::new("/Library/Application Support/com.mateoki.vproxy.tun"),
        0o755,
    )?;
    secure_root_dir(&paths.runtime, 0o700)?;
    for name in GEO_FILES {
        let target = paths.runtime.join(name);
        if let Some(sha) = manifest.geodata.get(*name) {
            let source = paths.install.join("geodata").join(name);
            let _ = root_file(&source)?;
            if file_sha256(&source)? != *sha {
                bail!("Installed geodata digest mismatch");
            }
            fs::copy(source, &target)?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
        } else if target.exists() {
            fs::remove_file(target)?;
        }
    }
    secure_root_dir(&paths.socket_dir, 0o755)?;
    if paths.socket.exists() {
        fs::remove_file(&paths.socket)?;
    }
    let listener = UnixListener::bind(&paths.socket)?;
    fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o600))?;
    let socket_c = CString::new(paths.socket.as_os_str().as_encoded_bytes())?;
    if unsafe { libc::chown(socket_c.as_ptr(), uid, gid) } != 0 {
        return Err(io::Error::last_os_error()).context("Assign helper socket owner");
    }
    listener.set_nonblocking(true)?;
    unsafe {
        libc::signal(
            libc::SIGTERM,
            stop_signal as *const () as libc::sighandler_t,
        );
        libc::signal(libc::SIGINT, stop_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let mut running: Option<Running> = None;
    while !STOP_HELPER.load(Ordering::Relaxed) {
        if let Some(child) = running.as_mut() {
            let _ = child.poll();
            if child.status.is_none()
                && (process_instance(child.parent).ok() != Some(child.parent_instance)
                    || process_hash(child.parent).ok().as_ref() != Some(&own_hash))
            {
                let _ = child.stop();
            }
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(IO_TIMEOUT))?;
                stream.set_write_timeout(Some(IO_TIMEOUT))?;
                let response = (|| -> Result<Response> {
                    let pid = authenticated_peer(&stream, uid, &own_hash)?;
                    let request: Request = read_frame(&mut stream)?;
                    let stopping = matches!(&request, Request::Stop { .. });
                    match request {
                        Request::Hello {} => Ok(Response::Ready {
                            manifest: manifest.clone(),
                        }),
                        Request::Start {
                            manifest: requested,
                            config,
                            log,
                        } => {
                            if requested != manifest {
                                bail!("内核或程序已更新，请重新授权安装 TUN 后台服务");
                            }
                            if running
                                .as_mut()
                                .is_some_and(|child| child.poll().ok().flatten().is_none())
                            {
                                bail!("TUN kernel already belongs to an active GUI session");
                            }
                            let mut input = open_user_file(&user, &config, false)?;
                            if input.metadata()?.len() > MAX_CONFIG {
                                bail!("TUN configuration is too large");
                            }
                            let mut config_bytes = Vec::new();
                            Read::by_ref(&mut input)
                                .take(MAX_CONFIG + 1)
                                .read_to_end(&mut config_bytes)?;
                            if config_bytes.len() as u64 > MAX_CONFIG {
                                bail!("TUN configuration is too large");
                            }
                            let stdout = open_user_file(&user, &log, true)?;
                            let stderr = stdout.try_clone()?;
                            let target = paths.runtime.join("config.yaml");
                            fs::write(&target, config_bytes)?;
                            fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
                            let parent_instance = process_instance(pid)?;
                            let session = Uuid::new_v4().to_string();
                            let child = Command::new(paths.install.join("mihomo"))
                                .args(["-d"])
                                .arg(&paths.runtime)
                                .arg("-f")
                                .arg(&target)
                                .current_dir(&paths.runtime)
                                .env_clear()
                                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                                .stdin(Stdio::null())
                                .stdout(Stdio::from(stdout))
                                .stderr(Stdio::from(stderr))
                                .spawn()?;
                            let child_pid = child.id();
                            running = Some(Running {
                                session: session.clone(),
                                parent: pid,
                                parent_instance,
                                child,
                                status: None,
                            });
                            Ok(Response::Running {
                                session,
                                pid: child_pid,
                            })
                        }
                        Request::Status { session } | Request::Stop { session } => {
                            // launchd may have restarted the helper after a crash.
                            // A missing old session is stopped, never an immortal
                            // "running" child because a caller ignored an RPC error.
                            let Some(child) = running.as_mut() else {
                                return Ok(Response::Exited { status: 1 << 8 });
                            };
                            if child.parent != pid || child.session != session {
                                bail!("TUN session does not belong to this GUI");
                            }
                            if stopping {
                                let status = child.stop()?;
                                return Ok(Response::Exited {
                                    status: status.into_raw(),
                                });
                            }
                            match child.poll()? {
                                Some(status) => Ok(Response::Exited {
                                    status: status.into_raw(),
                                }),
                                None => Ok(Response::Running {
                                    session: child.session.clone(),
                                    pid: child.child.id(),
                                }),
                            }
                        }
                    }
                })()
                .unwrap_or_else(|error| Response::Error {
                    message: format!("{error:#}"),
                });
                let _ = write_frame(&mut stream, &response);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(30))
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    drop(running);
    let _ = fs::remove_file(&paths.socket);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ipc_rejects_oversized_frames_before_allocating() {
        let (mut server, mut client) = UnixStream::pair().unwrap();
        client
            .write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
            .unwrap();
        assert!(read_frame::<Request>(&mut server)
            .unwrap_err()
            .to_string()
            .contains("length"));
    }
    #[test]
    fn ipc_rejects_unknown_operations_and_fields() {
        assert!(serde_json::from_str::<Request>(r#"{"op":"Execute","cmd":"id"}"#).is_err());
        assert!(
            serde_json::from_str::<Request>(r#"{"op":"Hello","executable":"/tmp/core"}"#).is_err()
        );
    }
    #[test]
    fn paths_reject_traversal_sibling_prefix_and_relative_names() {
        let base = Path::new("/Users/example/Library/Application Support/vproxy");
        assert!(require_user_subpath(&base.join("run/config.yaml"), base).is_ok());
        for invalid in [
            "relative.yaml",
            "/Users/example/Library/Application Support/vproxy-other/config.yaml",
            "/Users/example/Library/Application Support/vproxy/../secret",
        ] {
            assert!(require_user_subpath(Path::new(invalid), base).is_err());
        }
    }
    #[test]
    fn account_paths_allow_current_and_legacy_data_only() {
        let user = account(unsafe { libc::getuid() }).unwrap();
        assert_eq!(
            user.data_dirs,
            [
                user.home.join("Library/Application Support/vproxy"),
                user.home.join("Library/Application Support/vpn-parser"),
            ]
        );
        for base in &user.data_dirs {
            assert!(require_account_subpath(&user, &base.join("kernels/mihomo/mihomo")).is_ok());
            assert!(require_account_subpath(&user, &base.join("run/config.yaml")).is_ok());
            assert!(require_account_subpath(&user, base).is_err());
            assert!(require_account_subpath(&user, &base.join("../secret")).is_err());
        }
        for path in [
            user.home
                .join("Library/Application Support/vproxy-other/run/config.yaml"),
            user.home
                .join("Library/Application Support/vpn-parser-other/run/config.yaml"),
            user.home.join("other/run/config.yaml"),
            PathBuf::from("relative/config.yaml"),
        ] {
            assert!(require_account_subpath(&user, &path).is_err());
        }
    }
    #[test]
    fn opened_files_reject_symlinks_and_special_files() {
        use std::os::unix::fs::symlink;
        let mut user = account(unsafe { libc::getuid() }).unwrap();
        let directory = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("macos-helper-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        user.data_dirs = [directory.join("vproxy"), directory.join("vpn-parser")];
        for base in &user.data_dirs {
            fs::create_dir_all(base).unwrap();
            fs::write(base.join("config.yaml"), "tun: { enable: true }").unwrap();
            assert!(open_user_file(&user, &base.join("config.yaml"), false).is_ok());
        }
        let directory = user.data_dirs[0].clone();
        let regular = directory.join("config.yaml");
        fs::write(&regular, "tun: { enable: true }").unwrap();
        assert!(open_user_file(&user, &regular, false).is_ok());
        symlink(&regular, directory.join("linked.yaml")).unwrap();
        assert!(open_user_file(&user, &directory.join("linked.yaml"), false).is_err());
        fs::create_dir(directory.join("nested")).unwrap();
        fs::write(directory.join("nested/log"), "").unwrap();
        symlink(directory.join("nested"), directory.join("linked-dir")).unwrap();
        assert!(open_user_file(&user, &directory.join("linked-dir/log"), true).is_err());
        let fifo = directory.join("pipe");
        let name = CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(open_user_file(&user, &fifo, false).is_err());
        assert!(open_user_file(&user, &directory.join("missing"), true).is_err());
        assert!(!directory.join("missing").exists());
        fs::remove_dir_all(directory.parent().unwrap()).unwrap();
    }
    #[test]
    fn authorized_shell_literals_quote_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(apple_quote("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }
    #[test]
    fn failed_install_transaction_restores_old_app_and_plist() {
        let root = std::env::temp_dir().join(format!("vproxy-rollback-test-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let mut paths = Paths::new(501);
        paths.install = root.join("installed");
        paths.plist = root.join("installed.plist");
        let stage = root.join("new-stage");
        let backup = root.join("previous-install");
        let plist_stage = root.join("new.plist");
        let plist_backup = root.join("previous.plist");
        fs::create_dir(&paths.install).unwrap();
        fs::write(paths.install.join("marker"), "old app").unwrap();
        fs::write(&paths.plist, "old plist").unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("marker"), "new app").unwrap();
        fs::write(&plist_stage, "new plist").unwrap();
        // Exercise the real EXIT trap as an ordinary user using only this test's
        // directory. No launchd operation is performed by the test.
        let mut script =
            install_transaction_prelude(&paths, &stage, &backup, &plist_stage, &plist_backup)
                .unwrap()
                .replace("/bin/launchctl", "/usr/bin/true");
        script.push_str(
            r#"vproxy_swapping=1
/bin/mv "$vproxy_install" "$vproxy_backup"
vproxy_old_install=1
/bin/mv "$vproxy_plist" "$vproxy_plist_backup"
vproxy_old_plist=1
/bin/mv "$vproxy_stage" "$vproxy_install"
vproxy_new_install=1
/bin/mv "$vproxy_plist_stage" "$vproxy_plist"
vproxy_new_plist=1
exit 17
"#,
        );
        let status = Command::new("/bin/sh")
            .args(["-c", &script])
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(17));
        assert_eq!(
            fs::read_to_string(paths.install.join("marker")).unwrap(),
            "old app"
        );
        assert_eq!(fs::read_to_string(&paths.plist).unwrap(), "old plist");
        assert!(!stage.exists());
        assert!(!backup.exists());
        assert!(!plist_stage.exists());
        assert!(!plist_backup.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn complete_installation_script_has_valid_shell_syntax() {
        let user = account(unsafe { libc::getuid() }).unwrap();
        let paths = Paths::new(user.uid);
        let manifest = Manifest {
            protocol: PROTOCOL,
            gui_sha256: "a".repeat(64),
            core_sha256: "b".repeat(64),
            geodata: BTreeMap::new(),
        };
        let script = installation_script(InstallScriptInputs {
            paths: &paths,
            user: &user,
            executable: Path::new(
                "/Users/tester's/Library/Application Support/vproxy/kernels/mihomo/mihomo",
            ),
            data_root: Path::new(
                "/Users/tester's/Library/Application Support/vproxy/kernels/mihomo",
            ),
            manifest: &manifest,
            bundle: Path::new("/Applications/VProxy's Test.app"),
            stage: Path::new("/tmp/vproxy's user staging"),
            staging_root: Path::new("/Library/PrivilegedHelperTools/.test-staging"),
            backup: Path::new("/Library/PrivilegedHelperTools/.test-backup"),
            plist_staging: Path::new("/Library/LaunchDaemons/test.plist.staging"),
            plist_backup: Path::new("/Library/LaunchDaemons/test.plist.backup"),
        })
        .unwrap();
        // -n parses the generated script without running any command or asking
        // for privileges. Include apostrophes/spaces in source paths above.
        assert!(Command::new("/bin/bash")
            .args(["-n", "-c", &script])
            .status()
            .unwrap()
            .success());
    }
    #[test]
    fn launchd_runs_only_fixed_installed_helper() {
        let paths = Paths::new(501);
        let plist = daemon_plist(&paths, 501, 20);
        assert!(plist.contains("/Library/PrivilegedHelperTools/com.mateoki.vproxy.tun.501/VProxyHelper.app/Contents/MacOS/vproxy"));
        assert!(plist.contains("--macos-tun-helper"));
        assert!(plist.contains(&format!(
            "<string>/usr/bin/arch</string><string>-{}</string>",
            helper_architecture()
        )));
        assert!(!plist.contains("/Users/"));
        assert!(!plist.contains("/bin/sh"));
    }
}
