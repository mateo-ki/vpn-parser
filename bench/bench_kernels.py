# -*- coding: utf-8 -*-
"""内核对比基准：每内核 10,000 次探针（250 监听 × 40 批，相继执行）。

链路：探针 → 内核 listener(mixed) → socks5 outbound → 本地 sink(meow all-DIRECT)
      → 本地 204 HTTP 服务。全链路真实建链+转发，无外部网络依赖。

指标：总耗时、成功数、内核进程内存峰值(WS)、CPU 秒数、IO 读写字节。
"""
import asyncio
import ctypes
import ctypes.wintypes as wt
import json
import os
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

KERNELS_DIR = r"D:\software\VPN Parser\data\kernels"
WORK = r"C:\Users\loukas\AppData\Local\Temp\kernel-bench"
SINK_PORT = 18080        # meow sink (socks/http mixed)
LOCAL_204_PORT = 18081   # local 204 target
LISTEN_BASE = 21000      # 每批 250 个监听端口，批间错开
PER_BATCH = 250
BATCHES = 40             # 250 * 40 = 10,000 probes per kernel
PROBE_TIMEOUT = 8.0

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

kernel_exe = {
    "mihomo": os.path.join(KERNELS_DIR, "mihomo", "mihomo.exe"),
    "sing-box": os.path.join(KERNELS_DIR, "sing-box", "sing-box.exe"),
    "xray": os.path.join(KERNELS_DIR, "xray", "xray.exe"),
    "shoes": os.path.join(KERNELS_DIR, "shoes", "shoes.exe"),
    "meow": os.path.join(KERNELS_DIR, "meow", "meow.exe"),
}

def sample_process(pid):
    """ctypes 直读进程 CPU/内存/IO，零依赖。"""
    k32 = ctypes.windll.kernel32
    h = k32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not h:
        return None
    try:
        ft_create, ft_exit, ft_kernel, ft_user = FILETIME(), FILETIME(), FILETIME(), FILETIME()
        if not k32.GetProcessTimes(h, ctypes.byref(ft_create), ctypes.byref(ft_exit),
                                   ctypes.byref(ft_kernel), ctypes.byref(ft_user)):
            return None
        pmc = PROCESS_MEMORY_COUNTERS(); pmc.cb = ctypes.sizeof(pmc)
        if not (k32.K32GetProcessMemoryInfo(h, ctypes.byref(pmc), pmc.cb)
                if hasattr(k32, "K32GetProcessMemoryInfo")
                else ctypes.windll.psapi.GetProcessMemoryInfo(h, ctypes.byref(pmc), pmc.cb)):
            return None
        io = IO_COUNTERS()
        if not k32.GetProcessIoCounters(h, ctypes.byref(io)):
            return None
        to_sec = lambda ft: (ft.dwHighDateTime << 32 | ft.dwLowDateTime) / 1e7
        return {
            "cpu": to_sec(ft_kernel) + to_sec(ft_user),
            "ws": pmc.WorkingSetSize,
            "peak_ws": pmc.PeakWorkingSetSize,
            "io_read": io.ReadTransferCount,
            "io_write": io.WriteTransferCount,
        }
    finally:
        k32.CloseHandle(h)

class Sampler:
    def __init__(self, pid):
        self.pid = pid
        self.stop = threading.Event()
        self.samples = []
        self.thread = threading.Thread(target=self._run, daemon=True)

    def _run(self):
        while not self.stop.is_set():
            s = sample_process(self.pid)
            if s:
                self.samples.append(s)
            self.stop.wait(0.2)

    def start(self):
        self.thread.start()

    def finish(self):
        self.stop.set()
        self.thread.join(timeout=1)
        if not self.samples:
            return {"peak_ws_mb": 0, "cpu_s": 0, "io_mb": 0, "max_ws_mb": 0}
        return {
            "peak_ws_mb": max(s["peak_ws"] for s in self.samples) / 1e6,
            "max_ws_mb": max(s["ws"] for s in self.samples) / 1e6,
            "cpu_s": max(s["cpu"] for s in self.samples),
            "io_mb": (max(s["io_read"] for s in self.samples)
                      + max(s["io_write"] for s in self.samples)) / 1e6,
        }

class Handler204(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(204)
        self.end_headers()
    def log_message(self, *a):
        pass

def write_config(kernel, batch, base_port):
    """每批 250 个 mixed 监听 + socks5 出站（全部指向 sink）。"""
    d = os.path.join(WORK, f"{kernel}-{batch}")
    os.makedirs(d, exist_ok=True)
    listeners, outbounds, rules = [], [], []
    for i in range(PER_BATCH):
        port = base_port + i
        listeners.append({"name": f"in{i}", "type": "mixed", "port": port,
                          "listen": "127.0.0.1", "proxy": f"out{i}"})
        outbounds.append({"name": f"out{i}", "type": "socks5",
                          "server": "127.0.0.1", "port": SINK_PORT, "udp": False})
    cfg = None
    if kernel in ("mihomo", "meow"):
        cfg = {"mode": "rule", "log-level": "silent",
               "proxies": outbounds, "listeners": listeners, "rules": ["MATCH,DIRECT"]}
        path = os.path.join(d, "config.yaml")
        import json as _j
        text = _j.dumps(cfg, ensure_ascii=False, indent=1)
    elif kernel == "sing-box":
        inb = [{"type": "mixed", "tag": f"in{i}", "listen": "127.0.0.1",
                "listen_port": base_port + i} for i in range(PER_BATCH)]
        outs = [{"type": "socks", "tag": f"out{i}", "server": "127.0.0.1",
                 "server_port": SINK_PORT} for i in range(PER_BATCH)]
        outs.append({"type": "direct", "tag": "direct"})
        rr = [{"inbound": [f"in{i}"], "outbound": f"out{i}"} for i in range(PER_BATCH)]
        cfg = {"log": {"level": "info"}, "inbounds": inb, "outbounds": outs,
               "route": {"rules": rr, "final": "direct"}}
        path = os.path.join(d, "config.json")
        import json as _j
        text = _j.dumps(cfg, ensure_ascii=False, indent=1)
    elif kernel == "xray":
        inb = [{"tag": f"in{i}", "port": base_port + i, "listen": "127.0.0.1",
                "protocol": "socks", "settings": {"auth": "noauth", "udp": False}}
               for i in range(PER_BATCH)]
        outs = [{"tag": f"out{i}", "protocol": "socks",
                 "settings": {"servers": [{"address": "127.0.0.1", "port": SINK_PORT}]}}
                for i in range(PER_BATCH)]
        rr = [{"type": "field", "inboundTag": [f"in{i}"], "outboundTag": f"out{i}"}
              for i in range(PER_BATCH)]
        cfg = {"log": {"loglevel": "none"}, "inbounds": inb, "outbounds": outs,
               "routing": {"rules": rr}}
        path = os.path.join(d, "config.json")
        import json as _j
        text = _j.dumps(cfg, ensure_ascii=False, indent=1)
    else:  # shoes（手写 YAML，避免 pyyaml 依赖）
        docs = []
        for i in range(PER_BATCH):
            docs.append(
                f"- address: 127.0.0.1:{base_port + i}\n"
                f"  name: in{i}\n"
                f"  protocol:\n"
                f"    type: mixed\n"
                f"  rules:\n"
                f"  - action: allow\n"
                f"    masks: 0.0.0.0/0\n"
                f"    client_chain:\n"
                f"      address: 127.0.0.1:{SINK_PORT}\n"
                f"      protocol:\n"
                f"        type: socks\n"
            )
        path = os.path.join(d, "config.yaml")
        text = "\n".join(docs)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    return path

def kernel_command(kernel, cfg_path, home):
    exe = kernel_exe[kernel]
    if kernel == "mihomo":
        return [exe, "-d", home, "-f", cfg_path]
    if kernel == "meow":
        return [exe, "-d", home, "-f", cfg_path]
    if kernel == "sing-box":
        return [exe, "run", "-D", home, "-c", cfg_path]
    if kernel == "xray":
        return [exe, "run", "-c", cfg_path]
    if kernel == "shoes":
        return [exe, "--no-reload", cfg_path]
    raise ValueError(kernel)

async def probe_one(port):
    try:
        reader, writer = await asyncio.wait_for(
            asyncio.open_connection("127.0.0.1", port), timeout=PROBE_TIMEOUT)
        req = (f"GET http://127.0.0.1:{LOCAL_204_PORT}/204 HTTP/1.1\r\n"
               f"Host: 127.0.0.1:{LOCAL_204_PORT}\r\nConnection: close\r\n\r\n")
        writer.write(req.encode())
        await writer.drain()
        data = await asyncio.wait_for(reader.read(256), timeout=PROBE_TIMEOUT)
        writer.close()
        return data.startswith(b"HTTP/1.1 204") or data.startswith(b"HTTP/1.0 204")
    except Exception:
        return False

async def run_batch(kernel, batch, base_port, cfg_path, home):
    cmd = kernel_command(kernel, cfg_path, home)
    creation = 0x08000000  # CREATE_NO_WINDOW
    proc = subprocess.Popen(cmd, cwd=home, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, creationflags=creation)
    sampler = Sampler(proc.pid)
    sampler.start()
    ok, t0 = 0, time.time()
    try:
        # 等监听就绪：第一个端口 10s 内必须开
        deadline = time.time() + 15
        ready = False
        while time.time() < deadline:
            try:
                r, w = await asyncio.wait_for(
                    asyncio.open_connection("127.0.0.1", base_port), timeout=1)
                w.close()
                ready = True
                break
            except Exception:
                await asyncio.sleep(0.2)
        if ready:
            await asyncio.sleep(0.3)  # 让其余监听绑完
        if ready:
            results = await asyncio.gather(*[probe_one(base_port + i) for i in range(PER_BATCH)])
            ok = sum(1 for r in results if r)
    finally:
        time.sleep(0.3)  # 让最后一次采样进来
        sampler.finish()
        try:
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)],
                           capture_output=True, creationflags=creation)
        except Exception:
            proc.kill()
    return {"ok": ok, "elapsed": time.time() - t0, "metrics": sampler.finish()}

async def amain():
    only = sys.argv[1].split(",") if len(sys.argv) > 1 else None
    os.makedirs(WORK, exist_ok=True)
    sink_home = os.path.join(WORK, "sink-home")
    os.makedirs(sink_home, exist_ok=True)
    sink_cfg = os.path.join(sink_home, "config.yaml")
    with open(sink_cfg, "w", encoding="utf-8") as f:
        f.write(f"mixed-port: {SINK_PORT}\nmode: rule\nlog-level: silent\n"
                f"rules:\n  - MATCH,DIRECT\n")
    sink = subprocess.Popen([kernel_exe["meow"], "-d", sink_home, "-f", sink_cfg],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            creationflags=0x08000000)
    await asyncio.sleep(2)

    server = ThreadingHTTPServer(("127.0.0.1", LOCAL_204_PORT), Handler204)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    results = {}
    prev_path = os.path.join(WORK, "results.json")
    if os.path.exists(prev_path):
        with open(prev_path, encoding="utf-8") as f:
            results = json.load(f)
    for kernel in ["mihomo", "sing-box", "xray", "shoes", "meow"]:
        if only and kernel not in only:
            continue
        if not os.path.exists(kernel_exe[kernel]):
            results[kernel] = {"error": "binary missing"}
            continue
        home = os.path.join(WORK, f"{kernel}-home")
        os.makedirs(home, exist_ok=True)
        total_ok, total_t, agg = 0, 0.0, []
        t_kernel = time.time()
        for batch in range(BATCHES):
            base = LISTEN_BASE + batch * 1000
            cfg = write_config(kernel, batch, base)
            try:
                r = await run_batch(kernel, batch, base, cfg, home)
            except Exception as e:
                print(f"  batch {batch} error: {e}", flush=True)
                continue
            total_ok += r["ok"]
            total_t += r["elapsed"]
            agg.append(r["metrics"])
        wall = time.time() - t_kernel
        results[kernel] = {
            "probes": BATCHES * PER_BATCH,
            "ok": total_ok,
            "kernel_active_s": round(total_t, 1),
            "wall_s": round(wall, 1),
            "peak_ws_mb": round(max((a["peak_ws_mb"] for a in agg), default=0), 1),
            "max_ws_mb": round(max((a["max_ws_mb"] for a in agg), default=0), 1),
            "cpu_s": round(max((a["cpu_s"] for a in agg), default=0), 1),
            "io_mb": round(max((a["io_mb"] for a in agg), default=0), 1),
        }
        print(f"== {kernel}: {results[kernel]}", flush=True)
        with open(os.path.join(WORK, "results.json"), "w", encoding="utf-8") as f:
            json.dump(results, f, ensure_ascii=False, indent=1)

    server.shutdown()
    sink.kill()

if __name__ == "__main__":
    asyncio.run(amain())
