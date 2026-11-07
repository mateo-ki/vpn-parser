pub mod adapter;
pub mod compiler;
mod config;
pub mod ir;
pub mod manager;
pub mod mihomo;
pub mod rule_io;
pub mod shoes;
pub mod sing_box;
pub mod types;
pub mod tracer;
pub mod validator;
pub mod xray;
pub mod yaml_override;
pub mod script_override;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod vpcore_repro;

pub use config::*;
pub use manager::*;
pub use mihomo::*;
pub use tracer::*;
pub use types::*;
pub use yaml_override::CLASH_VERGE_RECOMMENDED_OVERRIDE;
