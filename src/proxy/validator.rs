use crate::kernel::{find_kernel_executable, managed_kernel_root, KernelKind};
use crate::proxy::compiler::CompiledArtifact;
use anyhow::{anyhow, Result};
use std::process::Command;
use uuid::Uuid;

#[allow(dead_code)]
pub struct KernelValidator;

#[allow(dead_code)]
impl KernelValidator {
    /// 校验编译好的配置工件在目标内核中语法与语义是否有效（使用内核 CLI 自检命令：-t / check / -test）
    pub fn validate_artifact(kernel: KernelKind, artifact: &CompiledArtifact) -> Result<()> {
        let text = artifact.to_string_pretty()?;
        Self::validate_config(kernel, &text)
    }

    /// 在临时隔离目录中写入配置，并调用内核自检命令进行非侵入式配置语法校验
    pub fn validate_config(kernel: KernelKind, config_text: &str) -> Result<()> {
        let Some(executable) = find_kernel_executable(kernel) else {
            tracing::info!(
                "未找到 {} 可执行文件，跳过内核静态语法验证",
                kernel.display_name()
            );
            return Ok(());
        };

        let temp_dir = std::env::temp_dir().join(format!(
            "vpn-parser-val-{}-{}",
            kernel.directory_name(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&temp_dir)?;

        let config_file_name = match kernel {
            KernelKind::Mihomo | KernelKind::Shoes | KernelKind::Meow | KernelKind::VpCore => "config.yaml",
            _ => "config.json",
        };
        let config_path = temp_dir.join(config_file_name);
        std::fs::write(&config_path, config_text)?;

        let mut cmd = Command::new(&executable);
        match kernel {
            KernelKind::Mihomo => {
                let geo_dir = crate::geo::mihomo_dir();
                let data_dir = if geo_dir.exists() {
                    geo_dir
                } else {
                    temp_dir.clone()
                };
                cmd.arg("-t")
                    .arg("-d")
                    .arg(data_dir)
                    .arg("-f")
                    .arg(&config_path);
            }
            KernelKind::SingBox => {
                cmd.arg("check")
                    .arg("-D")
                    .arg(&temp_dir)
                    .arg("-c")
                    .arg(&config_path);
            }
            KernelKind::Xray => {
                let asset_dir = managed_kernel_root().join(KernelKind::Xray.directory_name());
                let geo_dir = crate::geo::mihomo_dir();
                let effective_asset_dir = if asset_dir.join("geosite.dat").exists() {
                    asset_dir
                } else if geo_dir.join("geosite.dat").exists() {
                    geo_dir
                } else if asset_dir.exists() {
                    asset_dir
                } else {
                    temp_dir.clone()
                };
                cmd.current_dir(&effective_asset_dir);
                cmd.env("XRAY_LOCATION_ASSET", &effective_asset_dir);
                cmd.arg("run")
                    .arg("-test")
                    .arg("-c")
                    .arg(&config_path);
            }
            KernelKind::Shoes => {
                cmd.arg("check").arg(&config_path);
            }
            KernelKind::Meow => {
                // meow 与 mihomo 同构：geo 从 -d home 的发现路径加载
                let geo_dir = crate::geo::mihomo_dir();
                let data_dir = if geo_dir.exists() {
                    geo_dir
                } else {
                    temp_dir.clone()
                };
                cmd.arg("-t")
                    .arg("-d")
                    .arg(data_dir)
                    .arg("-f")
                    .arg(&config_path);
            }
            KernelKind::VpCore => {
                cmd.arg("validate")
                    .arg("-c")
                    .arg(&config_path);
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let output = match cmd.output() {
            Ok(out) => out,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(anyhow!("执行 {} 自检失败: {e}", kernel.display_name()));
            }
        };

        let _ = std::fs::remove_dir_all(&temp_dir);

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = format!("{stdout}\n{stderr}").trim().to_string();
            return Err(anyhow!(
                "{} 配置自检未通过: {}",
                kernel.display_name(),
                detail
            ));
        }

        Ok(())
    }
}
