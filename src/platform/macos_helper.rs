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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use uuid::Uuid;

const PROTOCOL: u32 = 2;
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
static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_GUI_IDENTITIES: usize = 64;

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
        session: SessionId,
        payload: StartPayload,
    },
    Status {
        session: SessionId,
    },
    Stop {
        session: SessionId,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "state", deny_unknown_fields)]
enum Response {
    Ready { manifest: Manifest, instance: String },
    Running { session: SessionId, process: ProcessIdentity },
    Exited { session: SessionId, status: i32 },
    SessionLost { instance: String },
    Error { message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SessionId {
    instance: String,
    id: String,
    sequence: u64,
    owner: PeerIdentity,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StartPayload {
    manifest: Manifest,
    config: PathBuf,
    log: PathBuf,
    config_sha256: String,
}

fn verify_config_digest(bytes: &[u8], expected: &str) -> Result<()> {
    if format!("{:x}", Sha256::digest(bytes)) != expected {
        bail!("TUN configuration changed after the Start request was prepared");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ProcessIdentity {
    pid: u32,
    instance: Option<(u64, u64)>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct PeerIdentity {
    pid: libc::pid_t,
    instance: (u64, u64),
}

#[derive(Clone)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnerLiveness { Alive, Dead, Unknown }

fn owner_liveness(owner: PeerIdentity) -> OwnerLiveness {
    match process_instance(owner.pid) {
        Ok(instance) if instance == owner.instance => OwnerLiveness::Alive,
        Ok(_) => OwnerLiveness::Dead,
        Err(_) => {
            // Signal zero only queries existence. Permission/errors do not
            // establish death; only ESRCH permits retiring this old identity.
            if unsafe { libc::kill(owner.pid, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            { OwnerLiveness::Dead } else { OwnerLiveness::Unknown }
        }
    }
}

fn with_live_owner<T>(owner: PeerIdentity,
    inspect: impl FnOnce(PeerIdentity) -> OwnerLiveness,
    dispatch: impl FnOnce() -> Result<T>) -> Result<T>
{
    if inspect(owner) != OwnerLiveness::Alive {
        bail!("GUI process identity changed or could not be confirmed after reading the request");
    }
    dispatch()
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

fn prepare_accepted_stream(stream: &UnixStream) -> io::Result<()> {
    // Darwin inherits O_NONBLOCK from the listener. Frame reads must wait for
    // the GUI to send its header/body; socket timeouts alone do not clear it.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))
}

#[derive(Debug)]
struct UnconfirmedReply;

impl std::fmt::Display for UnconfirmedReply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TUN helper request was sent but its reply could not be confirmed")
    }
}

impl std::error::Error for UnconfirmedReply {}

fn rpc_on_stream(stream: &mut UnixStream, request: &Request) -> Result<Response> {
    write_frame(stream, request)?;
    // A malformed frame is just as uncertain as a lost reply: the helper may
    // already have applied Start. Keep the decoding/I/O source for diagnostics.
    let response: Response = read_frame(stream).map_err(|error| error.context(UnconfirmedReply))?;
    match response {
        Response::Error { message } => Err(anyhow!(message)),
        response => Ok(response),
    }
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
    rpc_on_stream(&mut stream, request)
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
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array><string>/usr/bin/arch</string><string>-{}</string><string>{}</string><string>--macos-tun-helper</string><string>{uid}</string><string>{gid}</string></array><key>ProcessType</key><string>Interactive</string><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>AbandonProcessGroup</key><false/><key>ThrottleInterval</key><integer>5</integer><key>StandardOutPath</key><string>/dev/null</string><key>StandardErrorPath</key><string>/dev/null</string></dict></plist>\n", xml_escape(&paths.label), helper_architecture(), xml_escape(&exe.to_string_lossy()))
}

fn read_version_pipe(reader: &mut impl Read, output: &mut Vec<u8>) -> Result<()> {
    const MAX_VERSION_OUTPUT: usize = 64 * 1024;
    let mut buffer = [0u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if output.len() + count > MAX_VERSION_OUTPUT {
                    bail!("Mihomo 版本预检输出超过 64 KiB");
                }
                output.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

/// Execute only the immutable installed core's version command. This runs as
/// the ordinary GUI account and never loads a proxy configuration or creates TUN.
fn check_core_version(executable: &Path, timeout: Duration) -> Result<String> {
    let mut child = Command::new(executable)
        .args(KernelKind::Mihomo.version_args())
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Mihomo 版本预检无法执行，保留原 TUN 安装")?;
    let result = (|| -> Result<String> {
        let mut stdout = child.stdout.take().context("Missing version stdout")?;
        let mut stderr = child.stderr.take().context("Missing version stderr")?;
        for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
                return Err(io::Error::last_os_error()).context("Bound version output reads");
            }
        }
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        let status = loop {
            read_version_pipe(&mut stdout, &mut output)?;
            read_version_pipe(&mut stderr, &mut output)?;
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                bail!("Mihomo 版本预检超时，保留原 TUN 安装");
            }
            std::thread::sleep(Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())));
        };
        read_version_pipe(&mut stdout, &mut output)?;
        read_version_pipe(&mut stderr, &mut output)?;
        crate::kernel::classify_version_probe(
            status.success(),
            status.code(),
            &String::from_utf8_lossy(&output),
        )
        .map_err(anyhow::Error::msg)?
        .context("Mihomo 版本预检没有返回版本")
    })();
    if child.try_wait().ok().flatten().is_none() {
        child.kill().context("终止未完成的 Mihomo 版本预检失败")?;
        poll_child_until(Duration::from_secs(1), Duration::from_millis(10), || child.try_wait())?
            .context("Mihomo 版本预检终止后未在期限内回收")?;
    }
    result
}

/// Read-only installation check, invoked as the GUI user by the authorized
/// installer. The protected manifest, authenticated Hello and executable core
/// version must pass before the installer can discard the preceding installation.
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
        Response::Ready { manifest: active, .. } if active == manifest => {
            let core = paths.install.join("mihomo");
            let _ = root_file(&core)?;
            if file_sha256(&core)? != manifest.core_sha256 {
                bail!("Installed core manifest mismatch");
            }
            check_core_version(&core, Duration::from_secs(3))?;
            Ok(())
        }
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
    let mut script = format!(
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
"#,
        stage = path_quote(stage)?,
        backup = path_quote(backup)?,
        plist_stage = path_quote(plist_stage)?,
        plist_backup = path_quote(plist_backup)?,
        install = path_quote(&paths.install)?,
        plist = path_quote(&paths.plist)?,
        job = shell_quote(&format!("system/{}", paths.label))
    );
    // bootout may return while launchd is still removing the old job. Do not
    // replace its files or bootstrap a replacement until removal completes.
    script.push_str(r#"vproxy_stop_service() {
  vproxy_bootout_error=$(/bin/launchctl bootout "$vproxy_job" 2>&1) || true
  vproxy_removal_attempt=0
  while /bin/launchctl print "$vproxy_job" >/dev/null 2>&1; do
    if test "$vproxy_removal_attempt" -ge 50; then
      /bin/echo "Timed out waiting for TUN service removal: $vproxy_job" >&2
      if test -n "$vproxy_bootout_error"; then /usr/bin/printf '%s\n' "$vproxy_bootout_error" >&2; fi
      return 1
    fi
    vproxy_removal_attempt=$((vproxy_removal_attempt + 1))
    /bin/sleep 0.1
  done
  return 0
}
vproxy_bootstrap_service() {
  vproxy_bootstrap_attempt=0
  while true; do
    if vproxy_bootstrap_error=$(/bin/launchctl bootstrap system "$vproxy_plist" 2>&1); then
      return 0
    else
      vproxy_bootstrap_status=$?
    fi
    vproxy_bootstrap_attempt=$((vproxy_bootstrap_attempt + 1))
    # launchctl maps launchd's EINPROGRESS to the generic exit status 5.
    # Retry that status briefly even after print no longer sees the old job.
    if test "$vproxy_bootstrap_status" -ne 5 || test "$vproxy_bootstrap_attempt" -ge 50; then
      /usr/bin/printf '%s\n' "$vproxy_bootstrap_error" >&2
      /bin/echo "Could not register TUN service after $vproxy_bootstrap_attempt attempt(s): $vproxy_job" >&2
      return "$vproxy_bootstrap_status"
    fi
    /bin/sleep 0.1
  done
}
vproxy_cleanup() {
  vproxy_result=$?
  trap - EXIT HUP INT TERM
  if test "$vproxy_committed" -eq 0 && test "$vproxy_swapping" -eq 1; then
    if vproxy_stop_service; then
      if test "$vproxy_new_install" -eq 1; then /bin/mv "$vproxy_install" "$vproxy_stage" || /bin/echo 'Could not move the failed TUN installation aside' >&2; fi
      if test "$vproxy_old_install" -eq 1; then /bin/mv "$vproxy_backup" "$vproxy_install" || /bin/echo "Could not restore the previous TUN installation; retained at $vproxy_backup" >&2; fi
      if test "$vproxy_new_plist" -eq 1; then /bin/rm -f "$vproxy_plist" || /bin/echo 'Could not remove the failed TUN service plist' >&2; fi
      if test "$vproxy_old_plist" -eq 1; then /bin/mv "$vproxy_plist_backup" "$vproxy_plist" || /bin/echo "Could not restore the previous TUN plist; retained at $vproxy_plist_backup" >&2; fi
      if test -f "$vproxy_plist"; then vproxy_bootstrap_service || /bin/echo 'Could not restart the previous TUN service' >&2; fi
    else
      /bin/echo "Could not stop the failed TUN service; previous files retained at $vproxy_backup and $vproxy_plist_backup" >&2
    fi
  fi
  # Recursive cleanup is limited to this invocation's generated root staging path.
  /bin/rm -rf "$vproxy_stage" || true
  /bin/rm -f "$vproxy_plist_stage" || true
  exit "$vproxy_result"
}
trap vproxy_cleanup EXIT
trap 'exit 1' HUP INT TERM
"#);
    Ok(script)
}

fn install_service_swap_script() -> &'static str {
    r#"vproxy_swapping=1
    vproxy_stop_service
    if test -e "$vproxy_install"; then /bin/mv "$vproxy_install" "$vproxy_backup"; vproxy_old_install=1; fi
    if test -e "$vproxy_plist"; then /bin/mv "$vproxy_plist" "$vproxy_plist_backup"; vproxy_old_plist=1; fi
    /bin/mv "$vproxy_stage" "$vproxy_install"
    vproxy_new_install=1
    /bin/mv "$vproxy_plist_stage" "$vproxy_plist"
    vproxy_new_plist=1
    vproxy_bootstrap_service
    /bin/launchctl kickstart "$vproxy_job"
"#
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

fn installation_readiness_script(checker_command: &str) -> String {
    format!(r#"vproxy_ready=0
vproxy_attempt=0
vproxy_check_error=''
vproxy_ready_deadline=$(( $(/bin/date +%s) + 8 ))
while test "$vproxy_attempt" -lt 40 && test "$(/bin/date +%s)" -lt "$vproxy_ready_deadline"; do
  if vproxy_check_error=$({checker_command} 2>&1); then vproxy_ready=1; break; fi
  vproxy_attempt=$((vproxy_attempt + 1))
  /bin/sleep 0.1
done
if test "$vproxy_ready" -ne 1; then
  /bin/echo 'TUN helper did not pass the authenticated installation/core version check' >&2
  if test -n "$vproxy_check_error"; then /usr/bin/printf '%s\n' "$vproxy_check_error" >&2; fi
  exit 1
fi
"#)
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
    script.push_str(install_service_swap_script());
    let checker = paths.install.join("VProxyHelper.app/Contents/MacOS/vproxy");
    let checker_command = format!("/usr/bin/sudo -u {} -- /usr/bin/arch -{} {} --macos-tun-helper-check {}", shell_quote(&format!("#{}", user.uid)), helper_architecture(), q(&checker)?, user.uid);
    script.push_str(&installation_readiness_script(&checker_command));
    script.push_str(r#"vproxy_committed=1
    # Remove the preceding snapshot only after authenticated Hello and core -v.
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
    start_impl(kernel, executable, data_root, config, log, true)
}

/// Automatic startup/recovery must never open an administrator prompt.
pub fn start_existing(
    kernel: KernelKind,
    executable: &Path,
    data_root: &Path,
    config: &Path,
    log: &Path,
) -> Result<MacChild> {
    start_impl(kernel, executable, data_root, config, log, false)
}

#[derive(Debug, PartialEq, Eq)]
enum HelperPreparation {
    Wait,
    Install,
}

fn helper_preparation(installed_matches: bool, allow_install: bool) -> Result<HelperPreparation> {
    if installed_matches {
        // launchd recreates /var/run sockets at boot and after helper crashes.
        // A single missed Hello must not reinstall an unchanged service.
        Ok(HelperPreparation::Wait)
    } else if allow_install {
        Ok(HelperPreparation::Install)
    } else {
        bail!("TUN 后台服务需要安装或更新授权，请在 VProxy 界面手动启用 TUN 完成一次管理员验证");
    }
}

fn installed_manifest_matches(paths: &Paths, expected: &Manifest) -> bool {
    let read = (|| -> Result<Manifest> {
        let mut bytes = Vec::new();
        root_file(&paths.install.join("manifest.json"))?
            .take(MAX_FRAME as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_FRAME {
            bail!("Installed manifest is too large");
        }
        Ok(serde_json::from_slice(&bytes)?)
    })();
    read.is_ok_and(|installed| installed == *expected)
}

fn wait_for_helper_ready(
    timeout: Duration,
    interval: Duration,
    mut probe: impl FnMut() -> bool,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if probe() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("TUN 后台服务暂未就绪：已安装的服务未响应，请检查系统设置中的后台项目后重试；未重复请求管理员授权");
        }
        std::thread::sleep(interval.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn start_rpc_error(error: anyhow::Error) -> anyhow::Error {
    if transport_error(&error) {
        error.context("TUN 后台服务暂未就绪：服务连接中断，请稍后重试")
    } else {
        error
    }
}

fn transport_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<UnconfirmedReply>().is_some() || error.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::WouldBlock
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::Interrupted
            )
        })
    })
}

fn start_impl(
    kernel: KernelKind,
    executable: &Path,
    data_root: &Path,
    config: &Path,
    log: &Path,
    allow_install: bool,
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
    let mut config_file = open_user_file(&user, config, false)?;
    if config_file.metadata()?.len() > MAX_CONFIG { bail!("TUN configuration is too large"); }
    let mut config_bytes = Vec::new();
    Read::by_ref(&mut config_file).take(MAX_CONFIG + 1).read_to_end(&mut config_bytes)?;
    if config_bytes.len() as u64 > MAX_CONFIG { bail!("TUN configuration is too large"); }
    let config_sha256 = format!("{:x}", Sha256::digest(&config_bytes));
    let _ = open_user_file(&user, log, true)?;
    let manifest = current_manifest(executable, data_root)?;
    let mut helper_instance = match rpc(&paths, &Request::Hello {}, IO_TIMEOUT) {
        Ok(Response::Ready { manifest: installed, instance }) if installed == manifest => Some(instance),
        _ => None,
    };
    if helper_instance.is_none() {
        if helper_preparation(installed_manifest_matches(&paths, &manifest), allow_install)?
            == HelperPreparation::Install
        {
            install(&paths, &user, executable, data_root, &manifest)?;
        }
        wait_for_helper_ready(
            Duration::from_secs(8),
            Duration::from_millis(100),
            || match rpc(&paths, &Request::Hello {}, IO_TIMEOUT) {
                Ok(Response::Ready { manifest: installed, instance }) if installed == manifest => {
                    helper_instance = Some(instance);
                    true
                }
                _ => false,
            },
        )?;
    }
    let sequence = SESSION_SEQUENCE.fetch_update(Ordering::SeqCst, Ordering::SeqCst,
        |previous| previous.checked_add(1)).map_err(|_| anyhow!("TUN session sequence exhausted"))? + 1;
    let session = SessionId {
        instance: helper_instance.context("TUN helper did not provide its instance identity")?,
        id: Uuid::new_v4().to_string(), sequence,
        owner: PeerIdentity { pid: unsafe { libc::getpid() },
            instance: process_instance(unsafe { libc::getpid() })? },
    };
    let payload = StartPayload { manifest, config: config.into(), log: log.into(), config_sha256 };
    let state = start_session_with_rpc(session, payload, |request|
        rpc(&paths, request, Duration::from_secs(3)))?;
    Ok(MacChild { paths, state })
}

pub struct MacChild {
    paths: Paths,
    state: ClientSession,
}

struct ClientSession {
    session: SessionId,
    process: Option<ProcessIdentity>,
    exited: Option<ExitStatus>,
}

impl ClientSession {
    fn pending(session: SessionId) -> Self { Self { session, process: None, exited: None } }

    fn accept(&mut self, response: Response,
        gone: impl FnOnce(ProcessIdentity) -> io::Result<bool>) -> io::Result<Option<ExitStatus>>
    {
        match response {
            Response::Running { session, process }
                if session == self.session && process.pid > 0 && process.pid <= i32::MAX as u32 => {
                if self.process.is_some_and(|known| known != process) {
                    return Err(io::Error::other("TUN helper changed the process for an existing session"));
                }
                self.process = Some(process);
                Ok(None)
            }
            Response::Exited { session, status } if session == self.session => {
                let status = ExitStatus::from_raw(status);
                self.exited = Some(status);
                Ok(Some(status))
            }
            Response::SessionLost { instance } if instance != self.session.instance => {
                if let Some(process) = self.process {
                    if gone(process)? {
                        let status = ExitStatus::from_raw(1 << 8);
                        self.exited = Some(status);
                        return Ok(Some(status));
                    }
                }
                Err(io::Error::other("TUN 后台服务已重启，旧内核会话退出尚未确认；已保留会话，请勿重复启动"))
            }
            Response::Error { message } => Err(io::Error::other(message)),
            _ => Err(io::Error::other("Unexpected TUN helper session response")),
        }
    }
}

fn process_confirmed_gone(process: ProcessIdentity) -> io::Result<bool> {
    match process_instance(process.pid as libc::pid_t) {
        Ok(current) => Ok(process.instance.is_some_and(|expected| expected != current)),
        Err(_) => {
            if unsafe { libc::kill(process.pid as libc::pid_t, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            { Ok(true) } else { Ok(false) }
        }
    }
}

fn start_session_with_rpc(session: SessionId, payload: StartPayload,
    mut exchange: impl FnMut(&Request) -> Result<Response>) -> Result<ClientSession>
{
    let mut state = ClientSession::pending(session.clone());
    let request = Request::Start { session, payload };
    let mut uncertain = false;
    for _ in 0..2 {
        match exchange(&request) {
            Ok(Response::Error { message }) if !uncertain => return Err(anyhow!(message)),
            Ok(response) => {
                // A lost/restarted service or an invalid response must not throw
                // away the only handle that can cancel an uncertain Start.
                let _ = state.accept(response, |_| Ok(false));
                return Ok(state);
            }
            Err(error) if transport_error(&error) => uncertain = true,
            Err(error) if !uncertain => return Err(start_rpc_error(error)),
            Err(_) => return Ok(state),
        }
    }
    Ok(state)
}

impl MacChild {
    pub fn id(&self) -> Option<u32> {
        self.state.exited.is_none().then(|| self.state.process.map(|process| process.pid)).flatten()
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.state.exited {
            return Ok(Some(status));
        }
        let response = rpc(
            &self.paths,
            &Request::Status {
                session: self.state.session.clone(),
            },
            IO_TIMEOUT,
        )
        .map_err(io::Error::other)?;
        self.state.accept(response, process_confirmed_gone)
    }
    pub async fn kill(&mut self) -> io::Result<()> {
        if self.state.exited.is_some() {
            return Ok(());
        }
        let paths = self.paths.clone();
        let session = self.state.session.clone();
        let response = tokio::task::spawn_blocking(move ||
            rpc(&paths, &Request::Stop { session }, Duration::from_secs(3)))
            .await.map_err(io::Error::other)?.map_err(io::Error::other)?;
        self.state.accept(response, process_confirmed_gone)?
            .ok_or_else(|| io::Error::other("Helper did not reap the stopped kernel"))?;
        Ok(())
    }
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.state.exited { return Ok(status); }
            let paths = self.paths.clone();
            let session = self.state.session.clone();
            let response = tokio::task::spawn_blocking(move ||
                rpc(&paths, &Request::Status { session }, IO_TIMEOUT))
                .await.map_err(io::Error::other)?.map_err(io::Error::other)?;
            if let Some(status) = self.state.accept(response, process_confirmed_gone)? {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for MacChild {
    fn drop(&mut self) {
        if self.state.exited.is_none() {
            let _ = rpc(
                &self.paths,
                &Request::Stop {
                    session: self.state.session.clone(),
                },
                Duration::from_secs(3),
            );
        }
    }
}

struct Running {
    child: Child,
    identity: ProcessIdentity,
    status: Option<ExitStatus>,
}

fn poll_child_until(
    timeout: Duration,
    interval: Duration,
    mut poll: impl FnMut() -> io::Result<Option<ExitStatus>>,
) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = poll()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(interval.min(deadline.saturating_duration_since(Instant::now())));
    }
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
        if let Some(status) =
            poll_child_until(Duration::from_secs(2), Duration::from_millis(20), || {
                self.poll()
            })?
        {
            return Ok(status);
        }
        if let Err(error) = self.child.kill() {
            // The process may exit between the last poll and SIGKILL.
            if let Some(status) = self.poll()? {
                return Ok(status);
            }
            return Err(error);
        }
        poll_child_until(Duration::from_secs(2), Duration::from_millis(20), || {
            self.poll()
        })?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "TUN kernel did not exit within the bounded stop deadline",
            )
        })
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

trait SessionProcess {
    fn identity(&self) -> ProcessIdentity;
    fn poll(&mut self) -> io::Result<Option<ExitStatus>>;
    fn stop(&mut self) -> io::Result<ExitStatus>;
}

impl SessionProcess for Running {
    fn identity(&self) -> ProcessIdentity { self.identity }
    fn poll(&mut self) -> io::Result<Option<ExitStatus>> { Running::poll(self) }
    fn stop(&mut self) -> io::Result<ExitStatus> { Running::stop(self) }
}

struct ActiveSession<P> {
    owner: PeerIdentity,
    session: SessionId,
    payload: StartPayload,
    process: P,
}

struct PeerReplay {
    highest: u64,
    session: String,
    payload: Option<StartPayload>,
    status: Option<i32>,
}

/// Live/unknown GUI high-water marks are never evicted. A dead process instance
/// cannot authenticate again; all accepted connections are rechecked after read.
struct SessionRegistry<P> {
    instance: String,
    peers: BTreeMap<PeerIdentity, PeerReplay>,
    active: Option<ActiveSession<P>>,
}

impl<P: SessionProcess> SessionRegistry<P> {
    fn new(instance: String) -> Self {
        Self { instance, peers: BTreeMap::new(), active: None }
    }

    fn retire_dead_owners(&mut self, mut inspect: impl FnMut(PeerIdentity) -> OwnerLiveness) {
        let active_owner = self.active.as_ref().map(|active| active.owner);
        self.peers.retain(|owner, _| Some(*owner) == active_owner
            || inspect(*owner) != OwnerLiveness::Dead);
    }

    fn check_session(&self, owner: PeerIdentity, session: &SessionId) -> Result<Option<Response>> {
        if session.owner != owner {
            bail!("TUN session does not belong to this authenticated GUI process instance");
        }
        if session.instance != self.instance {
            return Ok(Some(Response::SessionLost { instance: self.instance.clone() }));
        }
        if session.sequence == 0 || Uuid::parse_str(&session.id).is_err() {
            bail!("Invalid TUN session identity");
        }
        Ok(None)
    }

    fn record_terminal(&mut self, owner: PeerIdentity, session: &SessionId, status: i32) {
        if let Some(record) = self.peers.get_mut(&owner) {
            if record.highest == session.sequence && record.session == session.id {
                record.status = Some(status);
            }
        }
    }

    fn poll_active(&mut self, owner_alive: impl FnOnce(PeerIdentity) -> bool) -> Result<()> {
        let terminal = if let Some(active) = self.active.as_mut() {
            let status = match active.process.poll()? {
                Some(status) => Some(status),
                None if !owner_alive(active.owner) => Some(active.process.stop()?),
                None => None,
            };
            status.map(|status| (active.owner, active.session.clone(), status.into_raw()))
        } else { None };
        if let Some((owner, session, status)) = terminal {
            self.record_terminal(owner, &session, status);
        }
        Ok(())
    }

    /// Reserve before spawning, or before answering an unknown Status/Stop.
    /// Unknown sessions become canceled: a delayed Start can never arrive later
    /// and resurrect a session that the GUI already confirmed as stopped.
    fn reserve(&mut self, owner: PeerIdentity, session: &SessionId, payload: Option<&StartPayload>) -> Result<bool> {
        if !self.peers.contains_key(&owner) {
            if self.peers.len() >= MAX_GUI_IDENTITIES {
                bail!("TUN helper session history is full; restart the helper before another GUI session");
            }
            self.peers.insert(owner, PeerReplay {
                highest: 0, session: String::new(), payload: None, status: Some(1 << 8),
            });
        }
        let record = self.peers.get_mut(&owner).expect("inserted peer");
        if session.sequence < record.highest {
            return Ok(false);
        }
        if session.sequence == record.highest {
            if session.id != record.session {
                bail!("TUN session sequence was reused with a different identity");
            }
            if let (Some(previous), Some(requested)) = (&record.payload, payload) {
                if previous != requested {
                    bail!("TUN Start retry changed its request content");
                }
            }
            return Ok(false);
        }
        *record = PeerReplay {
            highest: session.sequence, session: session.id.clone(),
            payload: payload.cloned(), status: Some(1 << 8),
        };
        Ok(true)
    }

    fn terminal_response(&self, owner: PeerIdentity, session: &SessionId) -> Response {
        let status = self.peers.get(&owner).filter(|record|
            record.highest == session.sequence && record.session == session.id)
            .and_then(|record| record.status).unwrap_or(1 << 8);
        Response::Exited { session: session.clone(), status }
    }

    fn start(&mut self, owner: PeerIdentity, session: SessionId, payload: StartPayload,
        spawn: impl FnOnce(&StartPayload) -> Result<P>) -> Result<Response>
    {
        if let Some(response) = self.check_session(owner, &session)? { return Ok(response); }
        if let Some(active) = self.active.as_mut() {
            if active.session == session {
                if active.owner != owner { bail!("TUN session does not belong to this GUI process"); }
                if active.payload != payload { bail!("TUN Start retry changed its request content"); }
                return Ok(match active.process.poll()? {
                    Some(status) => Response::Exited { session, status: status.into_raw() },
                    None => Response::Running { session, process: active.process.identity() },
                });
            }
        }
        if !self.reserve(owner, &session, Some(&payload))? {
            return Ok(self.terminal_response(owner, &session));
        }
        if self.active.as_mut().is_some_and(|active| active.process.poll().map_or(true, |status| status.is_none())) {
            bail!("TUN kernel already belongs to an active GUI session");
        }
        let process = spawn(&payload)?;
        let identity = process.identity();
        self.peers.get_mut(&owner).expect("reserved peer").status = None;
        self.active = Some(ActiveSession { owner, session: session.clone(), payload, process });
        Ok(Response::Running { session, process: identity })
    }

    fn status_or_stop(&mut self, owner: PeerIdentity, session: SessionId, stopping: bool) -> Result<Response> {
        if let Some(response) = self.check_session(owner, &session)? { return Ok(response); }
        if let Some(active) = self.active.as_mut() {
            if active.session == session {
                if active.owner != owner { bail!("TUN session does not belong to this GUI process"); }
                let status = if stopping { Some(active.process.stop()?) } else { active.process.poll()? };
                let response = match status {
                    Some(status) => Response::Exited { session: session.clone(), status: status.into_raw() },
                    None => Response::Running { session: session.clone(), process: active.process.identity() },
                };
                if let Some(status) = status { self.record_terminal(owner, &session, status.into_raw()); }
                return Ok(response);
            }
        }
        self.reserve(owner, &session, None)?;
        Ok(self.terminal_response(owner, &session))
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
    let mut sessions = SessionRegistry::<Running>::new(Uuid::new_v4().to_string());
    while !STOP_HELPER.load(Ordering::Relaxed) {
        let _ = sessions.poll_active(|owner|
            process_instance(owner.pid).ok() == Some(owner.instance)
                && process_hash(owner.pid).ok().as_ref() == Some(&own_hash));
        match listener.accept() {
            Ok((mut stream, _)) => {
                prepare_accepted_stream(&stream)?;
                let response = (|| -> Result<Response> {
                    let pid = authenticated_peer(&stream, uid, &own_hash)?;
                    let owner = PeerIdentity { pid, instance: process_instance(pid)? };
                    let request: Request = read_frame(&mut stream)?;
                    // A peer can exit while the frame is being received. Never
                    // dispatch its buffered request under a recycled PID.
                    if authenticated_peer(&stream, uid, &own_hash)? != owner.pid {
                        bail!("GUI peer PID changed while reading request");
                    }
                    with_live_owner(owner, owner_liveness, || {
                    if sessions.peers.len() >= MAX_GUI_IDENTITIES {
                        sessions.retire_dead_owners(owner_liveness);
                    }
                    let stopping = matches!(&request, Request::Stop { .. });
                    match request {
                        Request::Hello {} => Ok(Response::Ready {
                            manifest: manifest.clone(),
                            instance: sessions.instance.clone(),
                        }),
                        Request::Start { session, payload } => {
                            if payload.manifest != manifest {
                                bail!("内核或程序已更新，请重新授权安装 TUN 后台服务");
                            }
                            sessions.start(owner, session, payload, |payload| {
                            let mut input = open_user_file(&user, &payload.config, false)?;
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
                            verify_config_digest(&config_bytes, &payload.config_sha256)?;
                            let stdout = open_user_file(&user, &payload.log, true)?;
                            let stderr = stdout.try_clone()?;
                            let target = paths.runtime.join("config.yaml");
                            fs::write(&target, config_bytes)?;
                            fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
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
                            let identity = ProcessIdentity {
                                pid: child.id(),
                                instance: process_instance(child.id() as libc::pid_t).ok(),
                            };
                            Ok(Running {
                                child,
                                identity,
                                status: None,
                            })
                            })
                        }
                        Request::Status { session } | Request::Stop { session } => {
                            sessions.status_or_stop(owner, session, stopping)
                        }
                    }
                    })
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
    drop(sessions);
    let _ = fs::remove_file(&paths.socket);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_installed_helper_waits_without_another_authorization() {
        assert_eq!(
            helper_preparation(true, true).unwrap(),
            HelperPreparation::Wait
        );
        assert_eq!(
            helper_preparation(true, false).unwrap(),
            HelperPreparation::Wait
        );
        assert_eq!(
            helper_preparation(false, true).unwrap(),
            HelperPreparation::Install
        );
        let error = helper_preparation(false, false).unwrap_err();
        assert!(error.to_string().contains("需要安装或更新授权"));
    }

    #[test]
    fn helper_wait_retries_transient_startup_and_reports_bounded_offline_failure() {
        let mut attempts = 0;
        wait_for_helper_ready(Duration::from_secs(1), Duration::ZERO, || {
            attempts += 1;
            attempts == 3
        })
        .unwrap();
        assert_eq!(attempts, 3);
        let error = wait_for_helper_ready(Duration::ZERO, Duration::ZERO, || false).unwrap_err();
        assert!(error.to_string().starts_with("TUN 后台服务暂未就绪"));
    }

    #[test]
    fn disconnected_start_is_retryable_but_authorization_errors_are_preserved() {
        let error = start_rpc_error(io::Error::from(io::ErrorKind::ConnectionRefused).into());
        assert!(error.to_string().starts_with("TUN 后台服务暂未就绪"));
        let message = "内核或程序已更新，请重新授权安装 TUN 后台服务";
        assert_eq!(start_rpc_error(anyhow!(message)).to_string(), message);
    }

    #[test]
    fn child_exit_poll_is_bounded_and_propagates_io_errors() {
        let mut polls = 0;
        let status = poll_child_until(Duration::ZERO, Duration::ZERO, || {
            polls += 1;
            Ok(None)
        })
        .unwrap();
        assert!(status.is_none());
        assert_eq!(polls, 1);
        let error = poll_child_until(Duration::ZERO, Duration::ZERO, || {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn running_stop_reaps_a_controlled_process_that_handles_sigterm() {
        use std::io::{BufRead, BufReader};
        let mut child = Command::new("/bin/sh")
            .args([
                "-c",
                "trap 'exit 0' TERM; printf 'ready\\n'; while :; do /bin/sleep 0.1; done",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut ready = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready, "ready\n");
        let mut running = Running {
            identity: ProcessIdentity { pid: child.id(), instance: None },
            child,
            status: None,
        };
        let status = running.stop().unwrap();
        assert!(status.success());
        assert_eq!(running.poll().unwrap(), Some(status));
    }
    #[test]
    fn accepted_nonblocking_listener_reads_a_delayed_fragmented_frame() {
        let root = std::env::temp_dir().join(format!("vpi-{}", Uuid::new_v4().simple()));
        fs::create_dir(&root).unwrap();
        let socket = root.join("s");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let (connected_tx, connected_rx) = std::sync::mpsc::channel();
        let (send_tx, send_rx) = std::sync::mpsc::channel();
        let sender = std::thread::spawn(move || {
            let mut client = UnixStream::connect(socket).unwrap();
            connected_tx.send(()).unwrap();
            send_rx.recv().unwrap();
            let body = serde_json::to_vec(&Request::Hello {}).unwrap();
            let length = (body.len() as u32).to_be_bytes();
            // The server has started reading before either header fragment or
            // the body arrives, exposing inherited nonblocking I/O on Darwin.
            std::thread::sleep(Duration::from_millis(50));
            client.write_all(&length[..2]).unwrap();
            std::thread::sleep(Duration::from_millis(25));
            client.write_all(&length[2..]).unwrap();
            std::thread::sleep(Duration::from_millis(25));
            client.write_all(&body).unwrap();
        });
        connected_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        prepare_accepted_stream(&stream).unwrap();
        let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFL) };
        send_tx.send(()).unwrap();
        let request = read_frame::<Request>(&mut stream);
        let sent = sender.join();
        drop(stream);
        drop(listener);
        fs::remove_dir_all(&root).unwrap();
        assert!(flags >= 0 && flags & libc::O_NONBLOCK == 0);
        sent.unwrap();
        assert!(matches!(request.unwrap(), Request::Hello {}));
    }

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
    struct InstallTransactionFixture {
        root: PathBuf,
        paths: Paths,
        stage: PathBuf,
        backup: PathBuf,
        plist_stage: PathBuf,
        plist_backup: PathBuf,
    }

    impl InstallTransactionFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("vproxy-install-test-{}", Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let mut paths = Paths::new(501);
            paths.install = root.join("installed");
            paths.plist = root.join("installed.plist");
            let fixture = Self {
                stage: root.join("new-stage"),
                backup: root.join("previous-install"),
                plist_stage: root.join("new.plist"),
                plist_backup: root.join("previous.plist"),
                paths,
                root,
            };
            fs::create_dir(&fixture.paths.install).unwrap();
            fs::write(fixture.paths.install.join("marker"), "old app").unwrap();
            fs::write(&fixture.paths.plist, "old plist").unwrap();
            fs::create_dir(&fixture.stage).unwrap();
            fs::write(fixture.stage.join("marker"), "new app").unwrap();
            fs::write(&fixture.plist_stage, "new plist").unwrap();
            fs::write(fixture.root.join("job"), "old app").unwrap();
            fs::write(fixture.root.join("busy-new"), "2").unwrap();
            let launchctl = fixture.root.join("launchctl");
            // Model asynchronous removal and launchctl's generic status 5.
            // Every operation is confined to this fixture; no launchd IPC occurs.
            fs::write(&launchctl, r#"#!/bin/sh
set -eu
fixture=$VPROXY_TEST_FIXTURE
case "$1" in
  bootout)
    test "$2" = system/com.mateoki.vproxy.tun.501
    test -f "$fixture/job" || exit 113
    /usr/bin/printf 'bootout:%s\n' "$(/bin/cat "$fixture/job")" >> "$fixture/events"
    /bin/echo 3 > "$fixture/pending"
    ;;
  print)
    test "$2" = system/com.mateoki.vproxy.tun.501
    test -f "$fixture/job" || exit 113
    job=$(/bin/cat "$fixture/job")
    marker=$(/bin/cat "$fixture/installed/marker")
    if test "$job" != "$marker"; then /bin/echo 'VIOLATION: files replaced before removal' >> "$fixture/events"; fi
    if test -f "$fixture/stall-removal"; then exit 0; fi
    if test -f "$fixture/pending"; then
      remaining=$(/bin/cat "$fixture/pending")
      if test "$remaining" -eq 0; then
        /bin/rm "$fixture/job" "$fixture/pending"
        /usr/bin/printf 'removed:%s\n' "$job" >> "$fixture/events"
        exit 113
      fi
      /bin/echo $((remaining - 1)) > "$fixture/pending"
      /usr/bin/printf 'pending:%s\n' "$job" >> "$fixture/events"
    fi
    ;;
  bootstrap)
    test "$2" = system
    test "$3" = "$fixture/installed.plist"
    marker=$(/bin/cat "$fixture/installed/marker")
    /usr/bin/printf 'bootstrap:%s\n' "$marker" >> "$fixture/events"
    if test -f "$fixture/job"; then
      /bin/echo 'VIOLATION: bootstrap before removal' >> "$fixture/events"
      /bin/echo 'Bootstrap failed: 5: mock job removal in progress' >&2
      exit 5
    fi
    if test "$marker" = 'new app'; then
      if test -f "$fixture/fail-new-status"; then
        status=$(/bin/cat "$fixture/fail-new-status")
        /bin/echo "Bootstrap failed: $status: mock registration error" >&2
        exit "$status"
      fi
      remaining=$(/bin/cat "$fixture/busy-new")
      if test "$remaining" -gt 0; then
        /bin/echo $((remaining - 1)) > "$fixture/busy-new"
        /bin/echo 'Bootstrap failed: 5: mock registration in progress' >&2
        exit 5
      fi
    fi
    /bin/cp "$fixture/installed/marker" "$fixture/job"
    /usr/bin/printf 'registered:%s\n' "$marker" >> "$fixture/events"
    ;;
  kickstart)
    test "$2" = system/com.mateoki.vproxy.tun.501
    test -f "$fixture/job"
    ;;
  *) exit 97 ;;
esac
"#).unwrap();
            fs::set_permissions(&launchctl, fs::Permissions::from_mode(0o700)).unwrap();
            fixture
        }

        fn run(&self, tail: &str) -> std::process::Output {
            let mut script = install_transaction_prelude(
                &self.paths,
                &self.stage,
                &self.backup,
                &self.plist_stage,
                &self.plist_backup,
            )
            .unwrap();
            script.push_str(install_service_swap_script());
            script.push_str(tail);
            // Use the production shell transaction with only launchctl and
            // sleeping substituted. The real filesystem swap and trap execute.
            let script = script
                .replace(
                    "/bin/launchctl",
                    &path_quote(&self.root.join("launchctl")).unwrap(),
                )
                .replace("/bin/sleep", "/usr/bin/true");
            Command::new("/bin/sh")
                .args(["-c", &script])
                .env("VPROXY_TEST_FIXTURE", &self.root)
                .output()
                .unwrap()
        }

        fn events(&self) -> String {
            fs::read_to_string(self.root.join("events")).unwrap()
        }

        fn fake_version_program(&self, body: &str) -> PathBuf {
            let executable = self.root.join("fake-mihomo");
            fs::write(&executable, format!("#!/bin/sh\ntest \"$1\" = -v || exit 97\n{body}\n")).unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            executable
        }

        fn assert_old_restored(&self) {
            assert_eq!(
                fs::read_to_string(self.paths.install.join("marker")).unwrap(),
                "old app"
            );
            assert_eq!(fs::read_to_string(&self.paths.plist).unwrap(), "old plist");
            assert_eq!(
                fs::read_to_string(self.root.join("job")).unwrap(),
                "old app"
            );
            assert!(!self.events().contains("VIOLATION"));
        }
    }

    impl Drop for InstallTransactionFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn installation_waits_for_removal_and_retries_busy_registration() {
        let fixture = InstallTransactionFixture::new();
        let output = fixture.run("vproxy_committed=1\n");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events = fixture.events();
        assert_eq!(events.matches("pending:old app").count(), 3);
        assert_eq!(events.matches("bootstrap:new app").count(), 3);
        assert!(
            events.find("removed:old app").unwrap() < events.find("bootstrap:new app").unwrap()
        );
        assert!(events.contains("registered:new app"));
        assert!(!events.contains("VIOLATION"));
        assert_eq!(
            fs::read_to_string(fixture.paths.install.join("marker")).unwrap(),
            "new app"
        );
        assert_eq!(
            fs::read_to_string(&fixture.paths.plist).unwrap(),
            "new plist"
        );
    }

    #[test]
    fn failed_install_transaction_waits_then_restores_old_app_and_plist() {
        let fixture = InstallTransactionFixture::new();
        let output = fixture.run("exit 17\n");
        assert_eq!(output.status.code(), Some(17));
        fixture.assert_old_restored();
        let events = fixture.events();
        assert_eq!(events.matches("pending:new app").count(), 3);
        assert!(
            events.find("removed:new app").unwrap() < events.find("bootstrap:old app").unwrap()
        );
        assert!(!fixture.stage.exists());
        assert!(!fixture.backup.exists());
        assert!(!fixture.plist_stage.exists());
        assert!(!fixture.plist_backup.exists());
    }

    #[test]
    fn permanent_bootstrap_errors_are_bounded_preserved_and_rolled_back() {
        for (status, attempts) in [(5, 50), (64, 1)] {
            let fixture = InstallTransactionFixture::new();
            fs::write(fixture.root.join("fail-new-status"), status.to_string()).unwrap();
            let output = fixture.run("vproxy_committed=1\n");
            assert_eq!(output.status.code(), Some(status));
            assert!(String::from_utf8_lossy(&output.stderr).contains(&format!(
                "Bootstrap failed: {status}: mock registration error"
            )));
            assert_eq!(
                fixture.events().matches("bootstrap:new app").count(),
                attempts
            );
            fixture.assert_old_restored();
        }
    }

    #[test]
    fn removal_timeout_keeps_installed_files_untouched() {
        let fixture = InstallTransactionFixture::new();
        fs::write(fixture.root.join("stall-removal"), "").unwrap();
        let output = fixture.run("vproxy_committed=1\n");
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("Timed out waiting for TUN service removal"));
        fixture.assert_old_restored();
        assert!(!fixture.events().contains("bootstrap:"));
        assert!(!fixture.backup.exists());
        assert!(!fixture.plist_backup.exists());
    }

    #[test]
    fn installed_core_version_check_accepts_version_and_rejects_bad_output_or_execution() {
        let fixture = InstallTransactionFixture::new();
        let core = fixture.fake_version_program("printf 'Mihomo Meta fixture-v1\\n'");
        assert_eq!(check_core_version(&core, Duration::from_secs(1)).unwrap(), "Mihomo Meta fixture-v1");
        for body in ["exit 0", "printf 'mock unsupported architecture\\n' >&2; exit 7"] {
            let core = fixture.fake_version_program(body);
            assert!(check_core_version(&core, Duration::from_secs(1)).is_err());
        }
        fs::write(&core, "not an executable format").unwrap();
        assert!(check_core_version(&core, Duration::from_secs(1)).unwrap_err().to_string().contains("无法执行"));
        fs::set_permissions(&core, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(check_core_version(&core, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn stalled_core_version_check_is_bounded_and_reaps_its_fixture_child() {
        let fixture = InstallTransactionFixture::new();
        let pid_file = fixture.root.join("version.pid");
        let core = fixture.fake_version_program(&format!(
            "printf '%s\\n' \"$$\" > {}\nexec /bin/sleep 30", path_quote(&pid_file).unwrap(),
        ));
        let started = Instant::now();
        let error = check_core_version(&core, Duration::from_millis(200)).unwrap_err();
        assert!(error.to_string().contains("超时"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid = fs::read_to_string(pid_file).unwrap().trim().parse::<libc::pid_t>().unwrap();
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ECHILD));
    }

    #[test]
    fn core_version_check_limits_output_without_a_pipe_deadlock() {
        let fixture = InstallTransactionFixture::new();
        let core = fixture.fake_version_program("while :; do printf '0123456789abcdef0123456789abcdef'; done");
        let error = check_core_version(&core, Duration::from_secs(1)).unwrap_err();
        assert!(error.to_string().contains("64 KiB"), "{error:#}");
    }

    #[test]
    fn failed_core_check_before_commit_restores_the_previous_installation() {
        let fixture = InstallTransactionFixture::new();
        let checker = fixture.root.join("fake-checker");
        fs::write(&checker, "#!/bin/sh\nprintf 'mock Hello passed but core version failed\\n' >&2\nexit 7\n").unwrap();
        fs::set_permissions(&checker, fs::Permissions::from_mode(0o700)).unwrap();
        let tail = format!("{}vproxy_committed=1\n", installation_readiness_script(&path_quote(&checker).unwrap()));
        let output = fixture.run(&tail);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("core version failed"));
        fixture.assert_old_restored();
        assert!(!fixture.backup.exists());
        assert!(!fixture.plist_backup.exists());
    }

    #[test]
    fn installation_readiness_deadline_stops_retrying_before_commit() {
        let fixture = InstallTransactionFixture::new();
        let clock = fixture.root.join("fake-date");
        let clock_called = fixture.root.join("date-called");
        fs::write(&clock, format!(
            "#!/bin/sh\nif test -e {}; then /bin/echo 109; else /usr/bin/touch {}; /bin/echo 100; fi\n",
            path_quote(&clock_called).unwrap(), path_quote(&clock_called).unwrap(),
        )).unwrap();
        fs::set_permissions(&clock, fs::Permissions::from_mode(0o700)).unwrap();
        let script = installation_readiness_script("/usr/bin/true")
            .replace("/bin/date", &path_quote(&clock).unwrap());
        let output = fixture.run(&format!("{script}vproxy_committed=1\n"));
        assert_eq!(output.status.code(), Some(1));
        fixture.assert_old_restored();
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
    fn launchd_uses_interactive_process_policy() {
        let plist = daemon_plist(&Paths::new(501), 501, 20);
        let mut parser = Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-", "--", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        parser
            .stdin
            .take()
            .unwrap()
            .write_all(plist.as_bytes())
            .unwrap();
        let output = parser.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(parsed["ProcessType"], "Interactive");
        // launchd must reap children in the helper's process group after SIGKILL.
        assert_eq!(parsed["AbandonProcessGroup"], false);
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

#[cfg(test)]
mod response_loss_tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Counts { spawned: usize, stopped: usize }

    struct FakeProcess {
        identity: ProcessIdentity,
        exited: Option<ExitStatus>,
        counts: Rc<RefCell<Counts>>,
    }

    impl SessionProcess for FakeProcess {
        fn identity(&self) -> ProcessIdentity { self.identity }
        fn poll(&mut self) -> io::Result<Option<ExitStatus>> { Ok(self.exited) }
        fn stop(&mut self) -> io::Result<ExitStatus> {
            if let Some(status) = self.exited { return Ok(status); }
            self.counts.borrow_mut().stopped += 1;
            let status = ExitStatus::from_raw(9);
            self.exited = Some(status);
            Ok(status)
        }
    }

    fn owner() -> PeerIdentity { PeerIdentity { pid: 1234, instance: (10, 20) } }
    fn session(instance: &str, sequence: u64) -> SessionId {
        SessionId { instance: instance.into(), id: Uuid::new_v4().to_string(), sequence, owner: owner() }
    }
    fn payload() -> StartPayload {
        StartPayload {
            manifest: Manifest { protocol: PROTOCOL, gui_sha256: "test-gui".into(),
                core_sha256: "test-core".into(), geodata: BTreeMap::new() },
            config: "/test-only/config.yaml".into(), log: "/test-only/core.log".into(),
            config_sha256: "test-config-sha256".into(),
        }
    }

    #[test]
    fn start_digest_checks_the_opened_config_content_without_exposing_it() {
        let bytes = b"test-only: original config";
        let digest = format!("{:x}", Sha256::digest(bytes));
        verify_config_digest(bytes, &digest).unwrap();
        let error = verify_config_digest(b"test-only: replaced config", &digest).unwrap_err();
        assert!(error.to_string().contains("configuration changed"));
        assert!(!error.to_string().contains("replaced config"));
    }
    fn spawn(counts: &Rc<RefCell<Counts>>) -> FakeProcess {
        counts.borrow_mut().spawned += 1;
        FakeProcess {
            identity: ProcessIdentity { pid: 4000 + counts.borrow().spawned as u32,
                instance: Some((100, counts.borrow().spawned as u64)) },
            exited: None, counts: counts.clone(),
        }
    }
    fn exchange(registry: &mut SessionRegistry<FakeProcess>, request: &Request,
        counts: &Rc<RefCell<Counts>>) -> Result<Response>
    {
        match request {
            Request::Start { session, payload } => registry.start(owner(), session.clone(),
                payload.clone(), |_| Ok(spawn(counts))),
            Request::Status { session } => registry.status_or_stop(owner(), session.clone(), false),
            Request::Stop { session } => registry.status_or_stop(owner(), session.clone(), true),
            Request::Hello {} => unreachable!("fault tests never contact a real helper"),
        }
    }

    #[test]
    fn start_reply_loss_retries_the_same_request_without_a_second_process() {
        let mut registry = SessionRegistry::new("helper-a".into());
        let counts = Rc::new(RefCell::new(Counts::default()));
        let requested = session("helper-a", 1);
        let mut attempts = 0;
        let state = start_session_with_rpc(requested.clone(), payload(), |request| {
            attempts += 1;
            let (mut client, mut server) = UnixStream::pair()?;
            write_frame(&mut client, request)?;
            let request = read_frame::<Request>(&mut server)?;
            let response = exchange(&mut registry, &request, &counts)?;
            if attempts == 1 {
                drop(server); // The Start was handled, but no response frame arrived.
            } else {
                write_frame(&mut server, &response)?;
            }
            read_frame::<Response>(&mut client)
        }).unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(counts.borrow().spawned, 1);
        assert_eq!(state.session, requested);
        assert_eq!(state.process.unwrap().pid, 4001);
    }

    #[test]
    fn all_start_replies_lost_still_leave_a_handle_for_status_and_stop() {
        let mut registry = SessionRegistry::new("helper-a".into());
        let counts = Rc::new(RefCell::new(Counts::default()));
        let requested = session("helper-a", 1);
        let mut state = start_session_with_rpc(requested.clone(), payload(), |request| {
            exchange(&mut registry, request, &counts)?;
            Err(io::Error::from(io::ErrorKind::TimedOut).into())
        }).unwrap();
        assert!(state.process.is_none());
        assert_eq!(counts.borrow().spawned, 1);
        let response = registry.status_or_stop(owner(), requested.clone(), false).unwrap();
        assert!(state.accept(response, |_| panic!("same helper must not probe OS processes")).unwrap().is_none());
        assert_eq!(state.process.unwrap().pid, 4001);
        let response = registry.status_or_stop(owner(), requested, true).unwrap();
        assert!(state.accept(response, |_| panic!("same helper must not probe OS processes")).unwrap().is_some());
        assert_eq!(counts.borrow().stopped, 1);
    }

    #[test]
    fn a_lost_start_request_is_canceled_before_any_delayed_delivery_can_spawn() {
        for stopping in [false, true] {
            let mut registry = SessionRegistry::new("helper-a".into());
            let counts = Rc::new(RefCell::new(Counts::default()));
            let requested = session("helper-a", 1);
            let mut state = start_session_with_rpc(requested.clone(), payload(), |_request|
                Err(io::Error::from(io::ErrorKind::BrokenPipe).into())).unwrap();
            let response = registry.status_or_stop(owner(), requested.clone(), stopping).unwrap();
            assert!(state.accept(response, |_| panic!("no OS process was started")).unwrap().is_some());
            let delayed = registry.start(owner(), requested.clone(), payload(), |_| Ok(spawn(&counts))).unwrap();
            assert!(matches!(delayed, Response::Exited { session, .. } if session == requested));
            assert_eq!(counts.borrow().spawned, 0);
        }
    }

    #[test]
    fn stop_reply_loss_and_old_replays_cannot_stop_or_restart_a_new_session() {
        let mut registry = SessionRegistry::new("helper-a".into());
        let counts = Rc::new(RefCell::new(Counts::default()));
        let old = session("helper-a", 1);
        let started = registry.start(owner(), old.clone(), payload(), |_| Ok(spawn(&counts))).unwrap();
        let mut state = ClientSession::pending(old.clone());
        state.accept(started, |_| panic!("same helper must not probe OS processes")).unwrap();
        let lost = registry.status_or_stop(owner(), old.clone(), true).unwrap();
        let lost_status = match lost { Response::Exited { status, .. } => status, _ => panic!() };
        let repeated = registry.status_or_stop(owner(), old.clone(), true).unwrap();
        assert!(matches!(repeated, Response::Exited { status, .. } if status == lost_status));
        assert!(state.accept(repeated, |_| panic!("same helper must not probe OS processes")).unwrap().is_some());
        assert_eq!(counts.borrow().stopped, 1);
        let new = session("helper-a", 2);
        registry.start(owner(), new.clone(), payload(), |_| Ok(spawn(&counts))).unwrap();
        assert!(matches!(registry.status_or_stop(owner(), old.clone(), true).unwrap(), Response::Exited { .. }));
        assert!(matches!(registry.start(owner(), old, payload(), |_| Ok(spawn(&counts))).unwrap(), Response::Exited { .. }));
        assert!(matches!(registry.status_or_stop(owner(), new, false).unwrap(), Response::Running { .. }));
        assert_eq!(counts.borrow().spawned, 2);
        assert_eq!(counts.borrow().stopped, 1);
    }

    #[test]
    fn retries_cannot_change_payload_or_cross_gui_process_instances() {
        let mut registry = SessionRegistry::new("helper-a".into());
        let counts = Rc::new(RefCell::new(Counts::default()));
        let requested = session("helper-a", 1);
        registry.start(owner(), requested.clone(), payload(), |_| Ok(spawn(&counts))).unwrap();
        let mut changed = payload();
        changed.config_sha256 = "another-node-content".into();
        assert!(registry.start(owner(), requested.clone(), changed, |_| Ok(spawn(&counts))).is_err());
        let recycled_pid = PeerIdentity { pid: owner().pid, instance: (99, 0) };
        assert!(registry.start(recycled_pid, requested.clone(), payload(), |_| Ok(spawn(&counts))).is_err());
        assert!(registry.status_or_stop(recycled_pid, requested, true).is_err());
        assert_eq!(counts.borrow().spawned, 1);
        assert_eq!(counts.borrow().stopped, 0);
    }

    #[test]
    fn helper_restart_never_adopts_an_old_session_or_falsely_confirms_unknown_exit() {
        let counts = Rc::new(RefCell::new(Counts::default()));
        let old = session("helper-a", 1);
        let mut restarted = SessionRegistry::new("helper-b".into());
        let response = restarted.start(owner(), old.clone(), payload(), |_| Ok(spawn(&counts))).unwrap();
        assert!(matches!(response, Response::SessionLost { .. }));
        let mut pending = ClientSession::pending(old.clone());
        assert!(pending.accept(response, |_| panic!("unknown PID must not be probed")).is_err());
        assert!(pending.exited.is_none());
        pending.process = Some(ProcessIdentity { pid: 4001, instance: Some((100, 1)) });
        assert!(pending.accept(Response::SessionLost { instance: "helper-b".into() }, |_| Ok(false)).is_err());
        assert!(pending.exited.is_none());
        assert!(pending.accept(Response::SessionLost { instance: "helper-b".into() }, |_| Ok(true)).unwrap().is_some());
        let new = session("helper-b", 2);
        restarted.start(owner(), new.clone(), payload(), |_| Ok(spawn(&counts))).unwrap();
        assert!(matches!(restarted.status_or_stop(owner(), old, true).unwrap(), Response::SessionLost { .. }));
        assert!(matches!(restarted.status_or_stop(owner(), new, false).unwrap(), Response::Running { .. }));
        assert_eq!(counts.borrow().spawned, 1);
        assert_eq!(counts.borrow().stopped, 0);
    }

    #[test]
    fn replay_history_is_bounded_and_never_evicts_cancellation_watermarks() {
        let mut registry = SessionRegistry::<FakeProcess>::new("helper-a".into());
        let counts = Rc::new(RefCell::new(Counts::default()));
        let first = session("helper-a", 1);
        for index in 0..MAX_GUI_IDENTITIES {
            let peer = PeerIdentity { pid: owner().pid + index as i32, ..owner() };
            let mut cancellation = first.clone();
            cancellation.owner = peer;
            registry.status_or_stop(peer, cancellation, true).unwrap();
        }
        let extra = PeerIdentity { pid: 9999, instance: (1, 0) };
        let mut extra_session = session("helper-a", 1);
        extra_session.owner = extra;
        assert!(registry.start(extra, extra_session, payload(), |_| Ok(spawn(&counts))).is_err());
        assert!(matches!(registry.start(owner(), first, payload(), |_| Ok(spawn(&counts))).unwrap(), Response::Exited { .. }));
        assert_eq!(registry.peers.len(), MAX_GUI_IDENTITIES);
        assert_eq!(counts.borrow().spawned, 0);
    }

    #[test]
    fn dead_gui_identities_can_be_retired_across_more_than_sixty_four_reopens() {
        let mut registry = SessionRegistry::<FakeProcess>::new("helper-a".into());
        let counts = Rc::new(RefCell::new(Counts::default()));
        for index in 0..MAX_GUI_IDENTITIES * 3 {
            // The previous GUI exited. The helper reaps its fake child, then a
            // newly authenticated GUI has a distinct process-instance identity.
            registry.poll_active(|_| false).unwrap();
            if registry.peers.len() >= MAX_GUI_IDENTITIES {
                registry.retire_dead_owners(|_| OwnerLiveness::Dead);
            }
            let peer = PeerIdentity { pid: 3000 + index as i32, instance: (index as u64, 1) };
            let mut requested = session("helper-a", 1);
            requested.owner = peer;
            registry.start(peer, requested, payload(), |_| Ok(spawn(&counts))).unwrap();
            assert!(registry.peers.len() <= MAX_GUI_IDENTITIES);
        }
        assert_eq!(counts.borrow().spawned, MAX_GUI_IDENTITIES * 3);
    }

    #[test]
    fn living_or_unverifiable_owners_are_never_evicted_to_make_room() {
        for liveness in [OwnerLiveness::Alive, OwnerLiveness::Unknown] {
            let mut registry = SessionRegistry::<FakeProcess>::new("helper-a".into());
            let counts = Rc::new(RefCell::new(Counts::default()));
            for index in 0..MAX_GUI_IDENTITIES {
                let peer = PeerIdentity { pid: 3000 + index as i32, instance: (1, 0) };
                let mut requested = session("helper-a", 1);
                requested.owner = peer;
                registry.status_or_stop(peer, requested, true).unwrap();
            }
            registry.retire_dead_owners(|_| liveness);
            let extra = PeerIdentity { pid: 9999, instance: (1, 0) };
            let mut requested = session("helper-a", 1);
            requested.owner = extra;
            assert!(registry.start(extra, requested, payload(), |_| Ok(spawn(&counts))).is_err());
            assert_eq!(registry.peers.len(), MAX_GUI_IDENTITIES);
            assert_eq!(counts.borrow().spawned, 0);
        }
    }

    #[test]
    fn an_authenticated_gui_that_exits_during_frame_read_cannot_dispatch_late_start() {
        let mut registry = SessionRegistry::<FakeProcess>::new("helper-a".into());
        let counts = Rc::new(RefCell::new(Counts::default()));
        let requested = session("helper-a", 1);
        registry.status_or_stop(owner(), requested.clone(), true).unwrap();
        registry.retire_dead_owners(|_| OwnerLiveness::Dead);
        assert!(registry.peers.is_empty());
        let mut dispatched = false;
        let response = with_live_owner(owner(), |_| OwnerLiveness::Dead, || {
            dispatched = true;
            registry.start(owner(), requested, payload(), |_| Ok(spawn(&counts)))
        });
        assert!(response.is_err());
        assert!(!dispatched);
        assert_eq!(counts.borrow().spawned, 0);
        assert!(registry.peers.is_empty());
    }

    #[test]
    fn an_explicit_first_start_rejection_preserves_its_reason_without_a_pending_handle() {
        let mut attempts = 0;
        let message = "TUN configuration changed after the Start request was prepared";
        let result = start_session_with_rpc(session("helper-a", 1), payload(), |_| {
            attempts += 1;
            Ok(Response::Error { message: message.into() })
        });
        assert_eq!(attempts, 1);
        let error = match result { Err(error) => error, Ok(_) => panic!("explicit rejection must not create a handle") };
        assert_eq!(error.to_string(), message);
    }

    #[test]
    fn malformed_rpc_reply_retries_or_retains_the_same_session_and_decoder_source() {
        for invalid_length in [false, true] {
            for all_replies_lost in [false, true] {
                let (deliver, incoming) = std::sync::mpsc::channel::<UnixStream>();
                let worker = std::thread::spawn(move || {
                    let mut registry = SessionRegistry::new("helper-a".into());
                    let counts = Rc::new(RefCell::new(Counts::default()));
                    for attempt in 0..2 {
                        let mut stream = incoming.recv().unwrap();
                        let request = read_frame::<Request>(&mut stream).unwrap();
                        let response = exchange(&mut registry, &request, &counts).unwrap();
                        if attempt == 0 || all_replies_lost {
                            if invalid_length {
                                stream.write_all(&0u32.to_be_bytes()).unwrap();
                            } else {
                                let invalid_json = b"{this is not a response}";
                                stream.write_all(&(invalid_json.len() as u32).to_be_bytes()).unwrap();
                                stream.write_all(invalid_json).unwrap();
                            }
                        } else {
                            write_frame(&mut stream, &response).unwrap();
                        }
                    }
                    let spawned = counts.borrow().spawned;
                    spawned
                });
                let requested = session("helper-a", 1);
                let mut attempts = 0;
                let state = start_session_with_rpc(requested.clone(), payload(), |request| {
                    attempts += 1;
                    let (mut client, server) = UnixStream::pair()?;
                    deliver.send(server).unwrap();
                    let response = rpc_on_stream(&mut client, request);
                    if attempts == 1 || all_replies_lost {
                        let error = response.as_ref().unwrap_err();
                        assert!(error.downcast_ref::<UnconfirmedReply>().is_some());
                        assert!(transport_error(error));
                        if !invalid_length {
                            assert!(error.downcast_ref::<serde_json::Error>().is_some());
                        }
                    }
                    response
                }).unwrap();
                assert_eq!(attempts, 2);
                assert_eq!(state.session, requested);
                assert_eq!(state.process.is_none(), all_replies_lost);
                assert_eq!(worker.join().unwrap(), 1);
            }
        }
    }

    #[test]
    fn a_real_rpc_error_frame_is_a_clear_rejection_with_its_original_message() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let message = "mock config digest mismatch";
        let worker = std::thread::spawn(move || {
            assert!(matches!(read_frame::<Request>(&mut server).unwrap(), Request::Start { .. }));
            write_frame(&mut server, &Response::Error { message: message.into() }).unwrap();
        });
        let mut attempts = 0;
        let result = start_session_with_rpc(session("helper-a", 1), payload(), |request| {
            attempts += 1;
            rpc_on_stream(&mut client, request)
        });
        let error = match result { Err(error) => error, Ok(_) => panic!("clear Error frame must reject Start") };
        assert_eq!(error.to_string(), message);
        assert!(!transport_error(&error));
        assert_eq!(attempts, 1);
        worker.join().unwrap();
    }

    #[test]
    fn failed_spawn_is_terminal_and_wrong_session_responses_are_not_adopted() {
        let mut registry = SessionRegistry::<FakeProcess>::new("helper-a".into());
        let counts = Rc::new(RefCell::new(Counts::default()));
        let requested = session("helper-a", 1);
        assert!(registry.start(owner(), requested.clone(), payload(), |_| bail!("mock spawn failure")).is_err());
        assert!(matches!(registry.start(owner(), requested.clone(), payload(), |_| Ok(spawn(&counts))).unwrap(), Response::Exited { .. }));
        let mut state = ClientSession::pending(requested);
        let wrong = Response::Running { session: session("helper-a", 2),
            process: ProcessIdentity { pid: 9999, instance: Some((100, 1)) } };
        assert!(state.accept(wrong, |_| panic!("wrong session must not be probed")).is_err());
        assert!(state.process.is_none());
        assert_eq!(counts.borrow().spawned, 0);
    }
}
