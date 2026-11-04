//! Explicit local integration probe: creates utun without default routes or DNS changes.
use super::macos_helper::{self, MacChild};
use crate::kernel::{managed_kernel_path, KernelKind};
use anyhow::{anyhow, ensure, Context, Result};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port())
}

fn fixture() -> Result<(PathBuf, Value, u16, String)> {
    let dir = crate::data_root()
        .join("run")
        .join(format!("macos-tun-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir)?;
    let mixed = free_port()?;
    let mut controller = free_port()?;
    while mixed == controller {
        controller = free_port()?;
    }
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let device = (230..255)
        .map(|i| format!("utun{i}"))
        .find(|name| {
            !Command::new("/sbin/ifconfig")
                .arg(name)
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .context("没有可用于独立测试的 utun 接口")?;
    let config = json!({
        "mixed-port": mixed, "bind-address": "127.0.0.1", "allow-lan": false,
        "external-controller": format!("127.0.0.1:{controller}"), "secret": secret,
        "log-level": "info", "mode": "rule", "ipv6": false,
        "dns": {"enable": false},
        "tun": {"enable": true, "device": device, "stack": "gvisor", "auto-route": false,
            "auto-detect-interface": true, "inet4-address": ["198.19.233.1/30"]},
        "proxies": [], "proxy-groups": [{"name": "PROBE", "type": "select", "proxies": ["DIRECT", "REJECT"]}],
        "rules": ["MATCH,PROBE"]
    });
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join("mihomo.log"))?;
    fs::write(dir.join("config.yaml"), serde_yaml::to_string(&config)?)?;
    Ok((dir, config, controller, secret))
}

fn start(dir: &Path) -> Result<MacChild> {
    let core = managed_kernel_path(KernelKind::Mihomo);
    macos_helper::start(
        KernelKind::Mihomo,
        &core,
        core.parent().context("内核目录无效")?,
        &dir.join("config.yaml"),
        &dir.join("mihomo.log"),
    )
}

async fn request(
    client: &reqwest::Client,
    port: u16,
    secret: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let mut req = client
        .request(method, format!("http://127.0.0.1:{port}{path}"))
        .bearer_auth(secret);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let response = req.send().await?.error_for_status()?;
    let bytes = response.bytes().await?;
    if bytes.is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_slice(&bytes)?)
    }
}

async fn wait_ready(
    client: &reqwest::Client,
    port: u16,
    secret: &str,
    child: &mut MacChild,
) -> Result<()> {
    for _ in 0..100 {
        ensure!(child.try_wait()?.is_none(), "内核提前退出");
        if request(client, port, secret, reqwest::Method::GET, "/version", None)
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(anyhow!("测试内核控制器未就绪"))
}

pub fn run() -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } != 0,
        "请以普通用户运行自测，后台服务会单独授权"
    );
    let rt = tokio::runtime::Runtime::new()?;
    let (dir, config, port, secret) = fixture()?;
    let mut child = start(&dir)?;
    let outcome: Result<()> = rt.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()?;
        wait_ready(&client, port, &secret, &mut child).await?;
        let pid = child.id().context("后台服务未返回内核 PID")?;
        let output = Command::new("/bin/ps")
            .args(["-o", "uid=", "-p", &pid.to_string()])
            .output()?;
        ensure!(
            String::from_utf8_lossy(&output.stdout).trim() == "0",
            "内核不是 root 进程"
        );
        let device = config["tun"]["device"].as_str().unwrap();
        for _ in 0..40 {
            if Command::new("/sbin/ifconfig")
                .arg(device)
                .output()?
                .status
                .success()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        ensure!(
            Command::new("/sbin/ifconfig")
                .arg(device)
                .output()?
                .status
                .success(),
            "utun 未创建"
        );
        println!("PASS root core and {device} created; auto-route=false, DNS unchanged");
        let mut updated = config.clone();
        updated["mode"] = json!("global");
        request(
            &client,
            port,
            &secret,
            reqwest::Method::PUT,
            "/configs?force=true",
            Some(json!({"payload": serde_yaml::to_string(&updated)?})),
        )
        .await?;
        let current = request(
            &client,
            port,
            &secret,
            reqwest::Method::GET,
            "/configs",
            None,
        )
        .await?;
        ensure!(current["mode"] == "global", "配置热重载未生效");
        request(
            &client,
            port,
            &secret,
            reqwest::Method::PUT,
            "/proxies/PROBE",
            Some(json!({"name":"REJECT"})),
        )
        .await?;
        let group = request(
            &client,
            port,
            &secret,
            reqwest::Method::GET,
            "/proxies/PROBE",
            None,
        )
        .await?;
        ensure!(group["now"] == "REJECT", "节点组热切换未生效");
        println!("PASS inline config reload and proxy group switch");
        for enabled in [false, true] {
            request(
                &client,
                port,
                &secret,
                reqwest::Method::PATCH,
                "/configs",
                Some(json!({"tun":{"enable": enabled, "auto-route": false, "device":device}})),
            )
            .await?;
            let mut matches = false;
            for _ in 0..50 {
                if Command::new("/sbin/ifconfig")
                    .arg(device)
                    .output()?
                    .status
                    .success()
                    == enabled
                {
                    matches = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            ensure!(matches, "TUN 开关状态与虚拟网卡不一致: enable={enabled}");
        }
        println!("PASS TUN off/on from controller");
        child.kill().await?;
        child.wait().await?;
        for _ in 0..30 {
            if !Command::new("/sbin/ifconfig")
                .arg(device)
                .output()?
                .status
                .success()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        ensure!(
            !Command::new("/sbin/ifconfig")
                .arg(device)
                .output()?
                .status
                .success(),
            "停止后 utun 仍在"
        );
        println!("PASS stop reaps core and removes utun");
        Ok(())
    });
    if outcome.is_err() {
        let _ = rt.block_on(child.kill());
        let _ = rt.block_on(child.wait());
    }
    if let Err(ref e) = outcome {
        eprintln!(
            "{e:#}\n{}",
            fs::read_to_string(dir.join("mihomo.log")).unwrap_or_default()
        );
    }
    let _ = fs::remove_dir_all(&dir);
    outcome?;

    // The next start must reuse the installed service without another password prompt.
    let report = crate::data_root()
        .join("run")
        .join(format!("macos-parent-test-{}.json", uuid::Uuid::new_v4()));
    let status = Command::new(std::env::current_exe()?)
        .arg("--macos-tun-abandon-probe")
        .arg(&report)
        .status()?;
    ensure!(status.success(), "父进程退出测试未启动");
    let info: Value = serde_json::from_slice(&fs::read(&report)?)?;
    let pid = info["pid"].as_u64().context("测试 PID 缺失")? as i32;
    let mut gone = false;
    for _ in 0..80 {
        if unsafe { libc::kill(pid, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    ensure!(gone, "父进程消失后，后台服务没有回收内核");
    if let Some(device) = info["device"].as_str() {
        ensure!(
            !Command::new("/sbin/ifconfig")
                .arg(device)
                .output()?
                .status
                .success(),
            "父进程退出后 utun 未清理"
        );
    }
    if let Some(dir) = info["dir"].as_str() {
        let _ = fs::remove_dir_all(dir);
    }
    let _ = fs::remove_file(&report);
    println!("PASS service reused and abandoned core cleaned up");
    Ok(())
}

pub fn abandon(report: &Path) -> Result<()> {
    let (dir, config, port, secret) = fixture()?;
    let mut child = start(&dir)?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()?;
        wait_ready(&client, port, &secret, &mut child).await?;
        let device = config["tun"]["device"].as_str().unwrap();
        for _ in 0..40 {
            if Command::new("/sbin/ifconfig")
                .arg(device)
                .output()?
                .status
                .success()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(anyhow!("父进程退出测试未能创建 utun"))
    })?;
    fs::write(
        report,
        serde_json::to_vec(
            &json!({"pid": child.id(), "dir":dir, "device":config["tun"]["device"]}),
        )?,
    )?;
    // Intentionally skip destructors to verify daemon parent-death handling.
    std::process::exit(0)
}
