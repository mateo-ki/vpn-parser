# macOS Mihomo TUN

macOS 不支持 Linux 的 `setcap`。VProxy 使用独立的 root LaunchDaemon 运行 Mihomo；界面保持普通用户权限，继续管理节点、订阅、分流规则和 TUN 开关。

## 使用

1. 将发布的 `VProxy.app` 安装到“应用程序”，正常打开。
2. 选择 Mihomo 内核及节点，切换至 TUN 或系统代理 + TUN，然后启动代理。
3. 首次启动时，在 macOS 管理员授权窗口中安装后台服务。以后启动、停止、切换节点和调整 TUN 均在 VProxy 中操作，无需每次输入密码。

应用、内核或地理数据更新后，VProxy 会核对文件摘要，需要替换受保护副本时再次请求管理员授权。后台服务随系统启动；Mihomo 随代理启动和停止，界面进程消失时也会被回收。

目前仅支持 **Mihomo** 的 macOS TUN，须使用具有 hardened runtime 签名的 `.app`。便携归档中的独立二进制可以用于普通代理，但不能安装 TUN 服务。源码构建应按 `.github/workflows/build-macos.yml` 创建并签名应用包。

数据继续使用 `~/Library/Application Support/vproxy`；只有旧目录的升级用户继续使用 `~/Library/Application Support/vpn-parser`。服务不接受这两个目录之外的配置或日志路径，包括自定义诊断工作目录。另一个客户端使用 TUN 时，应先关闭其 TUN，避免同时接管路由。

## 服务与权限

`<uid>` 为当前用户的数字 UID，可用 `id -u` 查看。

| 对象 | 位置 |
| --- | --- |
| 服务名称 | `com.mateoki.vproxy.tun.<uid>` |
| LaunchDaemon | `/Library/LaunchDaemons/com.mateoki.vproxy.tun.<uid>.plist` |
| 应用、Mihomo 与地理数据的受保护副本 | `/Library/PrivilegedHelperTools/com.mateoki.vproxy.tun.<uid>/` |
| root 内核运行目录 | `/Library/Application Support/com.mateoki.vproxy.tun/<uid>/` |
| 本地控制套接字 | `/var/run/com.mateoki.vproxy.tun.<uid>/control.sock` |

安装时复制并校验应用、内核和地理数据，保留代码签名，文件归 root 所有。运行目录仅 root 可访问。控制套接字限制为对应用户，并通过内核提供的 UID、PID 与进程 CDHash 双向验证身份。IPC 只提供连接检查、启动、查询和停止，不能指定任意可执行程序。

配置与日志必须位于上述用户数据目录，逐级检查所有者、符号链接和文件类型。服务把配置复制至私有运行目录；配置热重载使用控制器内联内容，避免让 root 内核读取界面工作目录中的新路径。控制器保留原有本地端口和鉴权机制。

服务是持久的管理员权限组件，修改此模块时应同时审查安装脚本、IPC 身份校验、文件访问及内核退出清理。

## 验证

不需要管理员权限的回归与文件访问测试：

```bash
cargo test --lib macos_
cargo check --lib --no-default-features
```

显式本机集成探针通过 `macos-tun-probe` feature 编入，默认发行包不包含它：

```bash
cargo build --release --bin vproxy --features macos-tun-probe
# 将上述二进制打包、签名为 VProxy.app 后，以普通用户执行：
"/Applications/VProxy.app/Contents/MacOS/vproxy" --macos-tun-self-test
```

首次运行仍需管理员授权。探针使用独立端口和未使用的 `utun`，设置 `auto-route: false` 并关闭测试内核 DNS，不修改默认路由或系统 DNS。检查 root 内核与网卡创建、配置热重载、节点组切换、TUN 开关、停止回收、服务复用和父进程退出清理。成功项显示 `PASS`，失败时输出原因与测试内核日志。应在 VProxy 代理停止时运行。

## 卸载后台服务

先在 VProxy 中停止代理并退出，然后以当前用户执行：

```bash
vproxy_tun_uid="$(id -u)"
vproxy_tun_label="com.mateoki.vproxy.tun.${vproxy_tun_uid}"

sudo launchctl bootout "system/${vproxy_tun_label}"
sudo rm -f "/Library/LaunchDaemons/${vproxy_tun_label}.plist"
sudo rm -rf "/Library/PrivilegedHelperTools/${vproxy_tun_label}" \
  "/Library/Application Support/com.mateoki.vproxy.tun/${vproxy_tun_uid}" \
  "/var/run/${vproxy_tun_label}"
```

服务已停止或不存在时，`bootout` 可能提示找不到服务；确认 VProxy 已退出后可继续移除上述文件。此操作保留用户的节点、订阅和设置。以后再次启动 TUN 会重新请求安装授权。
