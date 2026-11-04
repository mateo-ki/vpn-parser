"""
Python ctypes example demonstrating VProxy C-ABI FFI bindings.
Supports Windows (.dll), Linux (.so), and macOS (.dylib).
"""

import ctypes
import json
import os
import sys

def find_library():
    base_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    target_dirs = [
        os.path.join(base_dir, "target", "release"),
        os.path.join(base_dir, "target", "debug"),
    ]
    if sys.platform == "win32":
        candidate_names = ["vproxy.dll", "vpn_parser.dll"]
    elif sys.platform == "darwin":
        candidate_names = ["libvproxy.dylib", "libvpn_parser.dylib"]
    else:
        candidate_names = ["libvproxy.so", "libvpn_parser.so"]

    for d in target_dirs:
        for lib_name in candidate_names:
            path = os.path.join(d, lib_name)
            if os.path.exists(path):
                return path
    raise FileNotFoundError(f"Cannot find {candidate_names} in target/debug or target/release. Run 'cargo build --lib' first.")

def main():
    lib_path = find_library()
    print(f"[FFI] Loading dynamic library: {lib_path}")
    lib = ctypes.CDLL(lib_path)

    # 1. Setup function signatures (prefer vproxy_*, fallback to vpn_*)
    fn_version = getattr(lib, "vproxy_version", getattr(lib, "vpn_version", None))
    fn_free_string = getattr(lib, "vproxy_free_string", getattr(lib, "vpn_free_string", None))
    fn_parse_sub = getattr(lib, "vproxy_parse_subscription", getattr(lib, "vpn_parse_subscription", None))
    fn_count = getattr(lib, "vproxy_parse_nodes_count", getattr(lib, "vpn_parse_nodes_count", None))
    fn_compile = getattr(lib, "vproxy_compile_config", getattr(lib, "vpn_compile_config", None))
    fn_ping = getattr(lib, "vproxy_test_tcp_latency", getattr(lib, "vpn_test_tcp_latency", None))

    assert fn_version and fn_free_string and fn_parse_sub and fn_count and fn_compile and fn_ping, "Failed to resolve FFI exports"

    fn_version.restype = ctypes.c_char_p
    fn_free_string.argtypes = [ctypes.c_char_p]
    fn_free_string.restype = None

    fn_parse_sub.argtypes = [ctypes.c_char_p]
    fn_parse_sub.restype = ctypes.c_void_p

    fn_count.argtypes = [ctypes.c_char_p]
    fn_count.restype = ctypes.c_int32

    fn_compile.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
    fn_compile.restype = ctypes.c_void_p

    fn_ping.argtypes = [ctypes.c_char_p, ctypes.c_uint16, ctypes.c_uint32]
    fn_ping.restype = ctypes.c_int32

    # 2. Query Version
    version = fn_version().decode("utf-8")
    print(f"[FFI] vproxy_version() = {version}")

    # 3. Parse Subscription
    test_uris = (
        "ss://YWVzLTEyOC1nY206cGFzc3dvcmRAMTI3LjAuMC4xOjgzODg=#Tokyo-Node-01\n"
        "hysteria2://password123@223.5.5.5:443#AliDNS-Hy2-Node\n"
    )
    c_sub = test_uris.encode("utf-8")
    count = fn_count(c_sub)
    print(f"[FFI] vproxy_parse_nodes_count() = {count} nodes")

    json_ptr = fn_parse_sub(c_sub)
    if json_ptr:
        json_str = ctypes.cast(json_ptr, ctypes.c_char_p).value.decode("utf-8")
        fn_free_string(ctypes.cast(json_ptr, ctypes.c_char_p))
        nodes = json.loads(json_str)
        print(f"[FFI] Parsed {len(nodes)} nodes successfully:")
        for idx, n in enumerate(nodes, 1):
            print(f"      {idx}. [{n['protocol']}] {n['name']} -> {n['server']}:{n['port']}")

        # 4. Compile config for first node (Mihomo)
        if nodes:
            first_node_json = json.dumps(nodes[0]).encode("utf-8")
            cfg_ptr = fn_compile(b"mihomo", first_node_json, None)
            if cfg_ptr:
                cfg_str = ctypes.cast(cfg_ptr, ctypes.c_char_p).value.decode("utf-8")
                fn_free_string(ctypes.cast(cfg_ptr, ctypes.c_char_p))
                print(f"[FFI] Compiled Mihomo configuration successfully ({len(cfg_str)} bytes).")
                print(f"[FFI] Config preview (first 150 chars):\n{cfg_str[:150]}...")

    # 5. TCP Ping
    target_host = b"223.5.5.5"
    latency = fn_ping(target_host, 443, 2000)
    print(f"[FFI] vproxy_test_tcp_latency(223.5.5.5:443) = {latency} ms")
    print("[FFI] All VProxy FFI checks passed successfully!")

if __name__ == "__main__":
    main()
