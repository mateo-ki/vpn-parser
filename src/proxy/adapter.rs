#![allow(dead_code)]
use super::{build_mihomo_config, MihomoConfigOptions};
use crate::kernel::{managed_kernel_root, KernelKind};
use crate::models::Node;
use crate::proxy::sing_box::build_sing_box_config;
use crate::proxy::xray::build_xray_config;
use crate::proxy::ProxyWorkDir;
use crate::proxy_probe::{ProxyProbeProgress, ProxyProbeResult};
use anyhow::Result;
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Semaphore;

pub trait KernelAdapter: Send + Sync {
    fn kind(&self) -> KernelKind;

    fn supports_protocol(&self, protocol: &str) -> bool {
        self.kind().supports_proxy_protocol(protocol)
    }

    fn build_config(&self, node: &Node, options: MihomoConfigOptions) -> Result<Value>;

    fn serialize_config(&self, config: &Value) -> Result<String>;

    fn configure_command(&self, command: &mut Command, work_dir: &ProxyWorkDir);

    #[allow(dead_code)]
    fn validation_args(&self, _work_dir: &ProxyWorkDir) -> Option<Vec<std::ffi::OsString>> {
        None
    }

    fn readiness_port(&self, options: &MihomoConfigOptions) -> u16;

    fn required_ports(&self, options: &MihomoConfigOptions) -> Vec<u16>;

    fn batch_size(&self) -> usize;

    fn verify_nodes<'a>(
        &'a self,
        nodes: Vec<Node>,
        timeout: Duration,
        progress: Option<&'a (dyn Fn(ProxyProbeProgress) + Send + Sync)>,
        network_limit: Arc<Semaphore>,
        batch_size: usize,
        conditional_isolation: bool,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<ProxyProbeResult>>> + Send + 'a>>;
}

pub struct MihomoAdapter;
pub struct SingBoxAdapter;
pub struct XrayAdapter;

pub static MIHOMO_ADAPTER: MihomoAdapter = MihomoAdapter;
pub static SING_BOX_ADAPTER: SingBoxAdapter = SingBoxAdapter;
pub static XRAY_ADAPTER: XrayAdapter = XrayAdapter;

pub fn kernel_adapter(kernel: KernelKind) -> &'static dyn KernelAdapter {
    match kernel {
        KernelKind::Mihomo => &MIHOMO_ADAPTER,
        KernelKind::SingBox => &SING_BOX_ADAPTER,
        KernelKind::Xray => &XRAY_ADAPTER,
    }
}

impl KernelAdapter for MihomoAdapter {
    fn kind(&self) -> KernelKind {
        KernelKind::Mihomo
    }

    fn build_config(&self, node: &Node, options: MihomoConfigOptions) -> Result<Value> {
        build_mihomo_config(node, options)
    }

    fn serialize_config(&self, config: &Value) -> Result<String> {
        Ok(serde_yaml::to_string(config)?)
    }

    fn configure_command(&self, command: &mut Command, work_dir: &ProxyWorkDir) {
        command
            .arg("-d")
            .arg(work_dir.data_root())
            .arg("-f")
            .arg(work_dir.config_path());
    }

    fn validation_args(&self, work_dir: &ProxyWorkDir) -> Option<Vec<std::ffi::OsString>> {
        Some(vec![
            "-t".into(),
            "-d".into(),
            work_dir.data_root().as_os_str().to_owned(),
            "-f".into(),
            work_dir.config_path().as_os_str().to_owned(),
        ])
    }

    fn readiness_port(&self, options: &MihomoConfigOptions) -> u16 {
        options.controller_port
    }

    fn required_ports(&self, options: &MihomoConfigOptions) -> Vec<u16> {
        let mut ports = vec![options.mixed_port, options.controller_port];
        for mapping in &options.port_mappings {
            if mapping.enabled && !ports.contains(&mapping.port) {
                ports.push(mapping.port);
            }
        }
        ports
    }

    fn batch_size(&self) -> usize {
        256
    }

    fn verify_nodes<'a>(
        &'a self,
        nodes: Vec<Node>,
        timeout: Duration,
        progress: Option<&'a (dyn Fn(ProxyProbeProgress) + Send + Sync)>,
        network_limit: Arc<Semaphore>,
        batch_size: usize,
        conditional_isolation: bool,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<ProxyProbeResult>>> + Send + 'a>> {
        Box::pin(async move {
            crate::proxy_probe::verify_nodes_with_mihomo_progress_limited(
                nodes,
                timeout,
                progress.map(|callback| move |state| callback(state)),
                network_limit,
                batch_size,
                conditional_isolation,
            )
            .await
        })
    }
}

impl KernelAdapter for SingBoxAdapter {
    fn kind(&self) -> KernelKind {
        KernelKind::SingBox
    }

    fn build_config(&self, node: &Node, options: MihomoConfigOptions) -> Result<Value> {
        build_sing_box_config(node, options)
    }

    fn serialize_config(&self, config: &Value) -> Result<String> {
        Ok(serde_json::to_string_pretty(config)?)
    }

    fn configure_command(&self, command: &mut Command, work_dir: &ProxyWorkDir) {
        command
            .arg("run")
            .arg("-D")
            .arg(work_dir.root())
            .arg("-c")
            .arg(work_dir.config_path());
    }

    fn validation_args(&self, work_dir: &ProxyWorkDir) -> Option<Vec<std::ffi::OsString>> {
        Some(vec![
            "check".into(),
            "-D".into(),
            work_dir.root().as_os_str().to_owned(),
            "-c".into(),
            work_dir.config_path().as_os_str().to_owned(),
        ])
    }

    fn readiness_port(&self, options: &MihomoConfigOptions) -> u16 {
        options.mixed_port
    }

    fn required_ports(&self, options: &MihomoConfigOptions) -> Vec<u16> {
        let mut ports = vec![options.mixed_port];
        for mapping in &options.port_mappings {
            if mapping.enabled {
                ports.push(mapping.port);
            }
        }
        ports
    }

    fn batch_size(&self) -> usize {
        64
    }

    fn verify_nodes<'a>(
        &'a self,
        nodes: Vec<Node>,
        timeout: Duration,
        progress: Option<&'a (dyn Fn(ProxyProbeProgress) + Send + Sync)>,
        network_limit: Arc<Semaphore>,
        _batch_size: usize,
        _conditional_isolation: bool,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<ProxyProbeResult>>> + Send + 'a>> {
        Box::pin(async move {
            crate::proxy::sing_box::verify_nodes_with_sing_box_progress_limited(
                nodes,
                timeout,
                progress.map(|callback| move |state| callback(state)),
                network_limit,
            )
            .await
        })
    }
}

impl KernelAdapter for XrayAdapter {
    fn kind(&self) -> KernelKind {
        KernelKind::Xray
    }

    fn build_config(&self, node: &Node, options: MihomoConfigOptions) -> Result<Value> {
        build_xray_config(node, options)
    }

    fn serialize_config(&self, config: &Value) -> Result<String> {
        Ok(serde_json::to_string_pretty(config)?)
    }

    fn configure_command(&self, command: &mut Command, work_dir: &ProxyWorkDir) {
        let asset_dir = managed_kernel_root().join(KernelKind::Xray.directory_name());
        let geo_dir = crate::geo::mihomo_dir();
        let effective_asset_dir = if asset_dir.join("geosite.dat").exists() {
            asset_dir
        } else if geo_dir.join("geosite.dat").exists() {
            geo_dir
        } else {
            asset_dir
        };
        command
            .current_dir(&effective_asset_dir)
            .env("XRAY_LOCATION_ASSET", effective_asset_dir)
            .arg("run")
            .arg("-c")
            .arg(work_dir.config_path());
    }

    fn validation_args(&self, work_dir: &ProxyWorkDir) -> Option<Vec<std::ffi::OsString>> {
        Some(vec![
            "run".into(),
            "-test".into(),
            "-c".into(),
            work_dir.config_path().as_os_str().to_owned(),
        ])
    }

    fn readiness_port(&self, options: &MihomoConfigOptions) -> u16 {
        options.mixed_port
    }

    fn required_ports(&self, options: &MihomoConfigOptions) -> Vec<u16> {
        let mut ports = vec![options.mixed_port];
        for mapping in &options.port_mappings {
            if mapping.enabled {
                ports.push(mapping.port);
            }
        }
        ports
    }

    fn batch_size(&self) -> usize {
        64
    }

    fn verify_nodes<'a>(
        &'a self,
        nodes: Vec<Node>,
        timeout: Duration,
        progress: Option<&'a (dyn Fn(ProxyProbeProgress) + Send + Sync)>,
        network_limit: Arc<Semaphore>,
        _batch_size: usize,
        _conditional_isolation: bool,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<ProxyProbeResult>>> + Send + 'a>> {
        Box::pin(async move {
            crate::proxy::xray::verify_nodes_with_xray_progress_limited(
                nodes,
                timeout,
                progress.map(|callback| move |state| callback(state)),
                network_limit,
            )
            .await
        })
    }
}
