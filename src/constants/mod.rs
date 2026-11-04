//! VProxy 全局常量系统
//!
//! 将散落在各模块的品牌、网络、平台底层与代理内核字符串常量集中抽象收敛，
//! 提供高内聚、易维护的单一真源。

pub mod brand;
pub mod network;
pub mod platform;
pub mod proxy;

pub use brand::*;
pub use network::*;
pub use platform::*;
pub use proxy::*;
