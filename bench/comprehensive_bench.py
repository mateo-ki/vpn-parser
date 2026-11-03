# -*- coding: utf-8 -*-
"""
VPN-Parser 全内核性能基准测试与测速全流水线压测工具 (带 30 秒冷却机制)
覆盖：
1. 配合 mihomo / sing-box / meow / vpcore 启动 100 节点代理，并发访问 100 网站（每内核测试完冷却 30 秒）
2. 测速全流程流水线（全量节点加载、DNS 预解析、TCP 预筛选、4 款内核真实握手，每内核测试完冷却 30 秒）
3. 逐阶段精细度量：耗时(ms)、物理常驻内存(Working Set MB & 峰值)、CPU 占用时间(秒)、IO 读写传输(KB/MB)
"""
import asyncio
import ctypes
import ctypes.wintypes as wt
import glob
import json
import os
import socket
import sqlite3
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

KERNELS = {
    "mihomo": r"D:\software\VPN Parser\data\kernels\mihomo\mihomo.exe",
    "sing-box": r"D:\software\VPN Parser\data\kernels\sing-box\sing-box.exe",
    "meow": r"D:\software\VPN Parser\data\kernels\meow\meow.exe",
    "vpcore": r"D:\software\VPN Parser\data\kernels\vpcore\vpcore.exe",
}

KERNEL_HOMES = {
    "mihomo": r"D:\software\VPN Parser\data\kernels\mihomo",
    "sing-box": r"D:\software\VPN Parser\data\kernels\sing-box",
    "meow": r"D:\software\VPN Parser\data\kernels\meow",
    "vpcore": r"D:\software\VPN Parser\data\kernels\vpcore",
}

SUBS_GLOB = r"data\subscriptions\*.db"
if not glob.glob(SUBS_GLOB):
    SUBS_GLOB = r"D:\software\VPN Parser\data\subscriptions\*.db"

WORK_DIR = os.path.join(os.environ.get("TEMP", r"C:\Users\loukas\AppData\Local\Temp"), "vpn_parser_bench_v3")
os.makedirs(WORK_DIR, exist_ok=True)

SINK_PORT = 28080
TARGET_HTTP_PORT = 28081

# Win32 Ctypes 性能采样结构体
class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
    _fields_ = [
        ("cb", wt.DWORD),
        ("PageFaultCount", wt.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
    ]

class IO_COUNTERS(ctypes.Structure):
    _fields_ = [
        ("ReadOperationCount", ctypes.c_ulonglong),
        ("WriteOperationCount", ctypes.c_ulonglong),
        ("OtherOperationCount", ctypes.c_ulonglong),
        ("ReadTransferCount", ctypes.c_ulonglong),
        ("WriteTransferCount", ctypes.c_ulonglong),
        ("OtherTransferCount", ctypes.c_ulonglong),
    ]

class FILETIME(ctypes.Structure):
    _fields_ = [("dwLowDateTime", wt.DWORD), ("dwHighDateTime", wt.DWORD)]

def sample_process(pid):
    """Win32 API 直读进程物理常驻内存/CPU/IO，微秒级精度"""
    k32 = ctypes.windll.kernel32
    h = k32.OpenProcess(0x1000, False, pid)
    if not h:
        return None
    try:
        ft_create, ft_exit, ft_kernel, ft_user = FILETIME(), FILETIME(), FILETIME(), FILETIME()
        if not k32.GetProcessTimes(h, ctypes.byref(ft_create), ctypes.byref(ft_exit),
                                   ctypes.byref(ft_kernel), ctypes.byref(ft_user)):
            return None
        pmc = PROCESS_MEMORY_COUNTERS()
        pmc.cb = ctypes.sizeof(pmc)
        if hasattr(k32, "K32GetProcessMemoryInfo"):
            k32.K32GetProcessMemoryInfo(h, ctypes.byref(pmc), pmc.cb)
        else:
            ctypes.windll.psapi.GetProcessMemoryInfo(h, ctypes.byref(pmc), pmc.cb)
        io = IO_COUNTERS()
        k32.GetProcessIoCounters(h, ctypes.byref(io))
        to_sec = lambda ft: (ft.dwHighDateTime << 32 | ft.dwLowDateTime) / 1e7
        return {
            "cpu": to_sec(ft_kernel) + to_sec(ft_user),
            "ws_mb": pmc.WorkingSetSize / (1024 * 1024),
            "peak_ws_mb": pmc.PeakWorkingSetSize / (1024 * 1024),
            "io_read_kb": io.ReadTransferCount / 1024,
            "io_write_kb": io.WriteTransferCount / 1024,
        }
    finally:
        k32.CloseHandle(h)

class ProcessSampler:
    def __init__(self, pid, interval=0.02):
        self.pid = pid
        self.interval = interval
        self.stop_event = threading.Event()
        self.samples = []
        self.thread = threading.Thread(target=self._run, daemon=True)

    def _run(self):
        while not self.stop_event.is_set():
            s = sample_process(self.pid)
            if s:
                self.samples.append(s)
            self.stop_event.wait(self.interval)

    def start(self):
        self.thread.start()

    def stop(self):
        self.stop_event.set()
        self.thread.join(timeout=1.0)
        final_sample = sample_process(self.pid)
        if final_sample:
            self.samples.append(final_sample)
        if not self.samples:
            return {
                "ws_mb": 0.0, "peak_ws_mb": 0.0, "cpu_s": 0.0,
                "io_read_kb": 0.0, "io_write_kb": 0.0
            }
        return {
            "ws_mb": self.samples[-1]["ws_mb"],
            "peak_ws_mb": max(s["peak_ws_mb"] for s in self.samples),
            "cpu_s": max(s["cpu"] for s in self.samples) - min(s["cpu"] for s in self.samples),
            "total_cpu_s": max(s["cpu"] for s in self.samples),
            "io_read_kb": max(0.0, self.samples[-1]["io_read_kb"] - self.samples[0]["io_read_kb"]),
            "io_write_kb": max(0.0, self.samples[-1]["io_write_kb"] - self.samples[0]["io_write_kb"]),
        }

# 100个模拟网站的高并发 HTTP Server（增大 Listen 队列避免高并发 SYN 丢弃重传延时）
class HighConcurrencyHTTPServer(ThreadingHTTPServer):
    request_queue_size = 256

class MockWebHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = f"<html><body>Website {self.path} - 200 OK Benchmark Target</body></html>".encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass

def wait_for_port(port, timeout=12.0):
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return True
        except OSError:
            time.sleep(0.04)
    return False

# 跨内核标准兼容 SOCKS5 出站终结代理
class Socks5Sink:
    def __init__(self, port):
        self.port = port
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind(("127.0.0.1", port))
        self.sock.listen(256)
        self.stop = False
        threading.Thread(target=self._accept_loop, daemon=True).start()

    def _accept_loop(self):
        while not self.stop:
            try:
                conn, _ = self.sock.accept()
                threading.Thread(target=self._handle, args=(conn,), daemon=True).start()
            except Exception:
                break

    def _handle(self, conn):
        dest = None
        try:
            conn.settimeout(4.0)
            conn.recv(256)
            conn.sendall(b"\x05\x00")
            req = conn.recv(256)
            if not req or len(req) < 4:
                conn.close()
                return
            atyp = req[3]
            if atyp == 1:
                addr = socket.inet_ntoa(req[4:8])
                port = int.from_bytes(req[8:10], "big")
            elif atyp == 3:
                l = req[4]
                addr = req[5:5+l].decode(errors="ignore")
                port = int.from_bytes(req[5+l:5+l+2], "big")
                if ":" in addr:
                    addr, p_str = addr.rsplit(":", 1)
                    if p_str.isdigit():
                        port = int(p_str)
            else:
                addr = "127.0.0.1"
                port = TARGET_HTTP_PORT

            dest = socket.create_connection((addr, port), timeout=4.0)
            conn.sendall(b"\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00")

            def relay(src, dst):
                try:
                    while True:
                        buf = src.recv(4096)
                        if not buf:
                            break
                        dst.sendall(buf)
                except Exception:
                    pass
                finally:
                    try: src.close()
                    except: pass
                    try: dst.close()
                    except: pass

            threading.Thread(target=relay, args=(conn, dest), daemon=True).start()
            relay(dest, conn)
        except Exception:
            try: conn.close()
            except: pass
            if dest:
                try: dest.close()
                except: pass

    def close(self):
        self.stop = True
        try:
            self.sock.close()
        except:
            pass

def cooldown(seconds=30, reason=""):
    print(f"\n[⏳ 静息冷却] {reason}，等待 {seconds} 秒以释放系统资源、清除网络栈并恢复空闲基线...", flush=True)
    for remaining in range(seconds, 0, -5):
        print(f"  ... 倒计时剩余 {remaining:2d}s", flush=True)
        time.sleep(5)
    print("  [✓] 冷却完成，准备启动下一项测试\n", flush=True)

def load_real_nodes(limit=100):
    files = glob.glob(SUBS_GLOB)
    nodes = []
    for f in files:
        try:
            conn = sqlite3.connect(f)
            cur = conn.cursor()
            cur.execute("SELECT id, name, protocol, server, port, config FROM nodes LIMIT 50")
            for row in cur.fetchall():
                try:
                    cfg = json.loads(row[5]) if row[5] else {}
                except Exception:
                    cfg = {}
                nodes.append({
                    "id": row[0],
                    "name": row[1],
                    "protocol": row[2],
                    "server": row[3],
                    "port": row[4],
                    "config": cfg
                })
                if len(nodes) >= limit:
                    return nodes
        except Exception:
            pass
    return nodes

async def http_get_via_proxy(sem, proxy_port, url_path):
    async with sem:
        t0 = time.perf_counter()
        try:
            reader, writer = await asyncio.wait_for(
                asyncio.open_connection("127.0.0.1", proxy_port), timeout=4.0
            )
            req = (
                f"GET http://127.0.0.1:{TARGET_HTTP_PORT}{url_path} HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{TARGET_HTTP_PORT}\r\n"
                f"User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) VPN-Parser/1.15.20\r\n"
                f"Connection: close\r\n\r\n"
            )
            writer.write(req.encode())
            await writer.drain()
            resp = await asyncio.wait_for(reader.read(512), timeout=4.0)
            writer.close()
            elapsed_ms = (time.perf_counter() - t0) * 1000.0
            ok = resp.startswith(b"HTTP/1.1 200") or resp.startswith(b"HTTP/1.0 200")
            return ok, elapsed_ms
        except Exception:
            return False, (time.perf_counter() - t0) * 1000.0

def build_100_node_config(kernel, nodes, port, tag="p1"):
    work = os.path.join(WORK_DIR, f"{tag}_{kernel}")
    os.makedirs(work, exist_ok=True)
    t0 = time.perf_counter()
    
    proxies = []
    for i, n in enumerate(nodes[:100]):
        proxies.append({
            "name": f"node_{i:02d}_{n['name'][:10].replace(':', '_').replace(' ', '_')}",
            "type": "socks5",
            "server": "127.0.0.1",
            "port": SINK_PORT,
            "udp": True,
        })
    proxy_names = [p["name"] for p in proxies]

    cfg_path = ""
    if kernel in ("mihomo", "meow", "vpcore"):
        cfg = {
            "mixed-port": port,
            "mode": "rule",
            "log-level": "warning",
            "ipv6": False,
            "proxies": proxies,
            "proxy-groups": [
                {
                    "name": "PROXY",
                    "type": "select",
                    "proxies": proxy_names,
                }
            ],
            "rules": [
                "MATCH,PROXY"
            ]
        }
        cfg_path = os.path.join(work, "config.yaml")
        text = json.dumps(cfg, ensure_ascii=False, indent=2)
        with open(cfg_path, "w", encoding="utf-8") as f:
            f.write(text)
    elif kernel == "sing-box":
        outbounds = [{"type": "selector", "tag": "PROXY", "outbounds": proxy_names}]
        for p in proxies:
            outbounds.append({
                "type": "socks",
                "tag": p["name"],
                "server": "127.0.0.1",
                "server_port": SINK_PORT,
            })
        outbounds.append({"type": "direct", "tag": "direct"})
        cfg = {
            "log": {"level": "warn"},
            "inbounds": [{
                "type": "mixed",
                "tag": "mixed-in",
                "listen": "127.0.0.1",
                "listen_port": port
            }],
            "outbounds": outbounds,
            "route": {
                "rules": [{"outbound": "PROXY"}],
                "final": "PROXY"
            }
        }
        cfg_path = os.path.join(work, "config.json")
        text = json.dumps(cfg, ensure_ascii=False, indent=2)
        with open(cfg_path, "w", encoding="utf-8") as f:
            f.write(text)

    elapsed_ms = (time.perf_counter() - t0) * 1000.0
    cfg_size_kb = os.path.getsize(cfg_path) / 1024.0
    return cfg_path, work, elapsed_ms, cfg_size_kb

# ============================================================================
# 测试 1：启动 100 节点代理 + 并发访问 100 个网站的阶段基准
# ============================================================================
async def bench_part1_kernel(kernel, nodes, port):
    print(f"\n=======================================================", flush=True)
    print(f">>> [测试 1: 100节点+100网站] 开始评测内核: {kernel} (监听端口: {port})", flush=True)
    print(f"=======================================================", flush=True)
    # 阶段 1：配置生成与编译
    t_cfg_start = time.perf_counter()
    self_before_cfg = sample_process(os.getpid())
    cfg_path, work, cfg_build_ms, cfg_size_kb = build_100_node_config(kernel, nodes, port, tag="p1")
    self_after_cfg = sample_process(os.getpid())
    cfg_cpu = max(0.0, self_after_cfg["cpu"] - self_before_cfg["cpu"]) if (self_before_cfg and self_after_cfg) else 0.0

    # 阶段 2：内核冷启动与端口监听就绪
    home = KERNEL_HOMES[kernel]
    exe = KERNELS[kernel]
    cmd = []
    if kernel == "mihomo":
        cmd = [exe, "-d", work, "-f", cfg_path]
    elif kernel == "sing-box":
        cmd = [exe, "run", "-D", work, "-c", cfg_path]
    elif kernel == "meow":
        cmd = [exe, "-d", home, "-f", cfg_path]
    elif kernel == "vpcore":
        cmd = [exe, "run", "-d", home, "-c", cfg_path]

    t_start = time.perf_counter()
    proc = subprocess.Popen(
        cmd, cwd=home, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        creationflags=0x08000000
    )
    ready = wait_for_port(port, timeout=12.0)
    startup_ms = (time.perf_counter() - t_start) * 1000.0
    initial_sample = sample_process(proc.pid)

    if not ready:
        proc.kill()
        return {"kernel": kernel, "error": "启动超时未监听"}

    # 阶段 3：并发代理访问 100 个网站
    sampler = ProcessSampler(proc.pid, interval=0.02)
    sampler.start()

    urls = [f"/site/{i}?category=tech&id={i * 13}&session=bench" for i in range(100)]
    sem = asyncio.Semaphore(16)
    t_req_start = time.perf_counter()
    tasks = [http_get_via_proxy(sem, port, u) for u in urls]
    results = await asyncio.gather(*tasks)
    total_req_ms = (time.perf_counter() - t_req_start) * 1000.0

    traffic_metrics = sampler.stop()
    ok_count = sum(1 for r in results if r[0])
    avg_latency_ms = sum(r[1] for r in results) / len(results) if results else 0.0

    # 阶段 4：停止与资源回收
    t_stop_start = time.perf_counter()
    proc.terminate()
    try:
        proc.wait(timeout=3.0)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    shutdown_ms = (time.perf_counter() - t_stop_start) * 1000.0

    res = {
        "kernel": kernel,
        "stage1_cfg_ms": round(cfg_build_ms, 2),
        "stage1_cfg_kb": round(cfg_size_kb, 1),
        "stage1_cpu_s": round(cfg_cpu, 3),
        "stage2_startup_ms": round(startup_ms, 1),
        "stage2_init_ws_mb": round(initial_sample["ws_mb"] if initial_sample else 0, 1),
        "stage2_init_cpu_s": round(initial_sample["cpu"] if initial_sample else 0, 3),
        "stage2_init_io_kb": round((initial_sample["io_read_kb"] + initial_sample["io_write_kb"]) if initial_sample else 0, 1),
        "stage3_req_total_ms": round(total_req_ms, 1),
        "stage3_avg_latency_ms": round(avg_latency_ms, 2),
        "stage3_ok": ok_count,
        "stage3_qps": round(100.0 / (total_req_ms / 1000.0), 1) if total_req_ms > 0 else 0,
        "stage3_peak_ws_mb": round(traffic_metrics["peak_ws_mb"], 1),
        "stage3_cpu_s": round(traffic_metrics["cpu_s"], 3),
        "stage3_io_kb": round(traffic_metrics["io_read_kb"] + traffic_metrics["io_write_kb"], 1),
        "stage4_shutdown_ms": round(shutdown_ms, 1),
    }
    print(f"  [1] 配置阶段: 耗时={res['stage1_cfg_ms']}ms, 配置体积={res['stage1_cfg_kb']}KB, CPU={res['stage1_cpu_s']}s", flush=True)
    print(f"  [2] 启动就绪: 耗时={res['stage2_startup_ms']}ms, 初始内存={res['stage2_init_ws_mb']}MB, CPU={res['stage2_init_cpu_s']}s, IO={res['stage2_init_io_kb']}KB", flush=True)
    print(f"  [3] 100站并发: 总耗时={res['stage3_req_total_ms']}ms, 均延={res['stage3_avg_latency_ms']}ms, 吞吐={res['stage3_qps']} req/s, 成功率={res['stage3_ok']}/100", flush=True)
    print(f"      流量期内存峰值={res['stage3_peak_ws_mb']}MB, 流量CPU={res['stage3_cpu_s']}s, 流量IO={res['stage3_io_kb']}KB", flush=True)
    print(f"  [4] 停止回收: 耗时={res['stage4_shutdown_ms']}ms (内存已全部归还操作系统)", flush=True)
    return res

# ============================================================================
# 测试 2：全量节点测速全流水线压测（DNS预解析、TCP预筛选、4款内核真实握手）
# ============================================================================
async def bench_part2_pipeline():
    print(f"\n=======================================================", flush=True)
    print(f">>> [测试 2] 开始全量节点测速全流水线深度压测...", flush=True)
    print(f"=======================================================", flush=True)

    # 阶段 1：全量节点加载与反序列化
    t0 = time.perf_counter()
    files = glob.glob(SUBS_GLOB)
    all_nodes = []
    total_db_bytes = 0
    self_before_db = sample_process(os.getpid())
    for f in files:
        total_db_bytes += os.path.getsize(f)
        try:
            conn = sqlite3.connect(f)
            cur = conn.cursor()
            cur.execute("SELECT id, name, protocol, server, port, config FROM nodes")
            for row in cur.fetchall():
                try:
                    cfg = json.loads(row[5]) if row[5] else {}
                except Exception:
                    cfg = {}
                all_nodes.append({
                    "id": row[0],
                    "name": row[1],
                    "protocol": row[2],
                    "server": row[3],
                    "port": row[4],
                    "config": cfg
                })
        except Exception:
            pass
    db_load_ms = (time.perf_counter() - t0) * 1000.0
    self_sample1 = sample_process(os.getpid())
    db_cpu = max(0.0, self_sample1["cpu"] - (self_before_db["cpu"] if self_before_db else 0.0))
    print(f"阶段 1 [全量节点加载与反序列化完成]: 节点总数={len(all_nodes)}, 耗时={db_load_ms:.1f}ms, "
          f"内存={self_sample1['ws_mb']:.1f}MB, CPU={db_cpu:.3f}s, 数据库读取IO={total_db_bytes/(1024*1024):.1f}MB", flush=True)

    # 阶段 2：并发 DNS 预解析
    dns_sample_hosts = list({n["server"] for n in all_nodes if not n["server"].replace(".", "").isdigit()})[:250]
    t_dns_start = time.perf_counter()
    self_before_dns = sample_process(os.getpid())
    def resolve_one(h):
        try:
            socket.gethostbyname(h)
            return True
        except Exception:
            return False

    from concurrent.futures import ThreadPoolExecutor
    with ThreadPoolExecutor(max_workers=64) as executor:
        results = list(executor.map(resolve_one, dns_sample_hosts))
        dns_resolved = sum(1 for r in results if r)
    dns_ms = (time.perf_counter() - t_dns_start) * 1000.0
    self_sample2 = sample_process(os.getpid())
    dns_cpu = max(0.0, self_sample2["cpu"] - (self_before_dns["cpu"] if self_before_dns else 0.0))
    dns_io = max(0.0, (self_sample2["io_read_kb"] + self_sample2["io_write_kb"]) -
                 (self_before_dns["io_read_kb"] + self_before_dns["io_write_kb"])) if self_before_dns else 0.0
    print(f"阶段 2 [并发 DNS 预解析完成]: 探测域名={len(dns_sample_hosts)}, 成功={dns_resolved}, "
          f"耗时={dns_ms:.1f}ms, 平均每域名={(dns_ms/len(dns_sample_hosts)):.2f}ms, "
          f"内存={self_sample2['ws_mb']:.1f}MB, CPU={dns_cpu:.3f}s, IO={dns_io:.1f}KB", flush=True)

    # 阶段 3：TCP Prefilter 预筛选
    tcp_sample = all_nodes[:500]
    t_tcp_start = time.perf_counter()
    self_before_tcp = sample_process(os.getpid())
    def tcp_probe(node):
        try:
            s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            s.settimeout(0.35)
            s.connect((node["server"], node["port"]))
            s.close()
            return True
        except Exception:
            return False

    with ThreadPoolExecutor(max_workers=128) as executor:
        tcp_res = list(executor.map(tcp_probe, tcp_sample))
        tcp_ok = sum(1 for r in tcp_res if r)
    tcp_ms = (time.perf_counter() - t_tcp_start) * 1000.0
    self_sample3 = sample_process(os.getpid())
    tcp_cpu = max(0.0, self_sample3["cpu"] - (self_before_tcp["cpu"] if self_before_tcp else 0.0))
    tcp_io = max(0.0, (self_sample3["io_read_kb"] + self_sample3["io_write_kb"]) -
                 (self_before_tcp["io_read_kb"] + self_before_tcp["io_write_kb"])) if self_before_tcp else 0.0
    print(f"阶段 3 [TCP Prefilter 预筛选完成]: 探测节点={len(tcp_sample)}, 存活={tcp_ok}, "
          f"耗时={tcp_ms:.1f}ms, 吞吐={len(tcp_sample)/(tcp_ms/1000.0):.0f} nodes/s, "
          f"内存={self_sample3['ws_mb']:.1f}MB, CPU={tcp_cpu:.3f}s, IO={tcp_io:.1f}KB", flush=True)

    # 阶段 4：真实内核批量握手测速对比（使用 100 节点标准测速批次交由 4 款内核真实压测，每款测试完冷却 30 秒）
    probe_batch = all_nodes[:100]
    kernel_probe_results = {}
    base_port = 28200
    kernel_list = ["mihomo", "sing-box", "meow", "vpcore"]
    for idx, k_name in enumerate(kernel_list):
        exe_path = KERNELS[k_name]
        port = base_port + idx * 10
        print(f"\n  -------------------------------------------------------------", flush=True)
        print(f"  正在执行阶段 4 内核握手测速: {k_name} (端口: {port})...", flush=True)
        print(f"  -------------------------------------------------------------", flush=True)
        cfg_path, work, _, _ = build_100_node_config(k_name, probe_batch, port, tag="p2")
        home = KERNEL_HOMES[k_name]
        cmd = []
        if k_name == "mihomo":
            cmd = [exe_path, "-d", work, "-f", cfg_path]
        elif k_name == "sing-box":
            cmd = [exe_path, "run", "-D", work, "-c", cfg_path]
        elif k_name == "meow":
            cmd = [exe_path, "-d", home, "-f", cfg_path]
        elif k_name == "vpcore":
            cmd = [exe_path, "run", "-d", home, "-c", cfg_path]

        proc = subprocess.Popen(
            cmd, cwd=home, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=0x08000000
        )
        wait_for_port(port, timeout=12.0)
        sampler = ProcessSampler(proc.pid, interval=0.02)
        sampler.start()

        # 对 100 个节点并发发送代理握手探测 (使用 Semaphore 16 稳定压测)
        sem = asyncio.Semaphore(16)
        t_batch_start = time.perf_counter()
        tasks = [http_get_via_proxy(sem, port, f"/probe/{i}") for i in range(100)]
        res = await asyncio.gather(*tasks)
        batch_total_ms = (time.perf_counter() - t_batch_start) * 1000.0

        metrics = sampler.stop()
        proc.terminate()
        try:
            proc.wait(timeout=2.0)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()

        success_count = sum(1 for r in res if r[0])
        kernel_probe_results[k_name] = {
            "batch_total_ms": round(batch_total_ms, 1),
            "per_node_ms": round(batch_total_ms / 100.0, 2),
            "peak_ws_mb": round(metrics["peak_ws_mb"], 1),
            "cpu_s": round(metrics["cpu_s"], 3),
            "io_kb": round(metrics["io_read_kb"] + metrics["io_write_kb"], 1),
            "success": success_count,
        }
        print(f"    -> {k_name} 握手结果: 100节点总耗时={kernel_probe_results[k_name]['batch_total_ms']}ms, "
              f"单节点平均={kernel_probe_results[k_name]['per_node_ms']}ms, 成功率={success_count}/100, "
              f"内存峰值={kernel_probe_results[k_name]['peak_ws_mb']}MB, "
              f"CPU={kernel_probe_results[k_name]['cpu_s']}s, "
              f"IO={kernel_probe_results[k_name]['io_kb']}KB", flush=True)

        # 每个内核测速完，停止 30 秒，再测速下一个（最后一个内核除外）
        if idx < len(kernel_list) - 1:
            cooldown(30, f"{k_name} 握手测速完成")

    return {
        "stage1_load": {
            "total_nodes": len(all_nodes),
            "elapsed_ms": round(db_load_ms, 1),
            "memory_mb": round(self_sample1["ws_mb"], 1),
            "cpu_s": round(db_cpu, 3),
            "io_mb": round(total_db_bytes / (1024 * 1024), 2),
        },
        "stage2_dns": {
            "sampled_domains": len(dns_sample_hosts),
            "resolved_count": dns_resolved,
            "elapsed_ms": round(dns_ms, 1),
            "avg_per_domain_ms": round(dns_ms / len(dns_sample_hosts), 2),
            "memory_mb": round(self_sample2["ws_mb"], 1),
            "cpu_s": round(dns_cpu, 3),
            "io_kb": round(dns_io, 1),
        },
        "stage3_tcp": {
            "sampled_nodes": len(tcp_sample),
            "alive_count": tcp_ok,
            "elapsed_ms": round(tcp_ms, 1),
            "throughput_nodes_per_sec": round(len(tcp_sample) / (tcp_ms / 1000.0), 1),
            "memory_mb": round(self_sample3["ws_mb"], 1),
            "cpu_s": round(tcp_cpu, 3),
            "io_kb": round(tcp_io, 1),
        },
        "stage4_kernel_probes": kernel_probe_results,
    }

async def main():
    print("================================================================")
    print("      VPN-Parser 代理内核与测速全流水线深度基准测试平台         ")
    print("        （含内核级 30 秒硬件资源完全冷却与隔离调度机制）        ")
    print("================================================================")

    # 1. 启动 Mock 100 站点 Web Server
    web_server = HighConcurrencyHTTPServer(("127.0.0.1", TARGET_HTTP_PORT), MockWebHandler)
    web_thread = threading.Thread(target=web_server.serve_forever, daemon=True)
    web_thread.start()
    print(f"[+] 目标 Web 服务已就绪: http://127.0.0.1:{TARGET_HTTP_PORT}")

    # 2. 启动 Local Sink 代理
    sink_proc = Socks5Sink(SINK_PORT)
    print(f"[+] 本地 Sink 真实代理已就绪: 127.0.0.1:{SINK_PORT}")

    # 3. 准备 100 个真实节点样本
    nodes = load_real_nodes(100)
    print(f"[+] 成功从订阅库中装载 100 个真实代理节点用于基准压测")

    # 运行测试 1：4款内核启动100节点代理并并发访问100网站（每内核测试完冷却30秒）
    p1_results = {}
    ports = {
        "mihomo": 28110,
        "sing-box": 28120,
        "meow": 28130,
        "vpcore": 28140,
    }
    kernel_list = ["mihomo", "sing-box", "meow", "vpcore"]
    for idx, k in enumerate(kernel_list):
        res = await bench_part1_kernel(k, nodes, ports[k])
        p1_results[k] = res
        # 每个内核测速完，停止 30 秒，再测速下一个
        if idx < len(kernel_list) - 1:
            cooldown(30, f"{k} 100节点+100站代理访问测试完成")

    # 两个大测试之间也进行 30 秒深度静息
    cooldown(30, "测试 1 全部内核压测完成，过渡至测试 2 全量测速流水线")

    # 运行测试 2：全量节点测速全流水线压测与4款内核握手横评（每内核测试完冷却30秒）
    p2_results = await bench_part2_pipeline()

    # 清理
    sink_proc.close()
    web_server.shutdown()

    # 结果输出并保存
    report = {
        "part1_100nodes_100sites": p1_results,
        "part2_full_pipeline": p2_results,
    }
    out_file = os.path.join(WORK_DIR, "benchmark_report.json")
    with open(out_file, "w", encoding="utf-8") as f:
        json.dump(report, f, ensure_ascii=False, indent=2)

    print("\n================================================================")
    print(f"[✓] 基准压测全部完成！报告已保存至: {out_file}")
    print("================================================================")

if __name__ == "__main__":
    asyncio.run(main())
