/**
 * @file vproxy.h
 * @brief C-ABI bindings for VProxy (Headless Core)
 * 
 * Cross-platform C bindings supporting Android JNI, iOS Swift/ObjC, 
 * HarmonyOS C/NAPI, Python ctypes, and Node.js FFI.
 */

#ifndef VPROXY_H
#define VPROXY_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Primary VProxy API ────────────────────────────────────────── */

/**
 * @brief Returns the version string of the VProxy library.
 * @return Statically allocated null-terminated string (do not free).
 */
const char* vproxy_version(void);

/**
 * @brief Frees a string previously allocated and returned by VProxy FFI functions.
 * @param ptr Pointer to the string to free. Passing NULL is safe.
 */
void vproxy_free_string(char* ptr);

/**
 * @brief Parses raw subscription text or node URIs into a JSON array string.
 * @param content Null-terminated string containing base64 subscription, Clash YAML, Sing-box JSON, or URIs.
 * @return JSON array of parsed nodes `[{"id":"...", ...}]` or error `{"error":"..."}`. Must be freed with vproxy_free_string().
 */
char* vproxy_parse_subscription(const char* content);

/**
 * @brief Returns the number of valid proxy nodes in the subscription.
 * @param content Null-terminated subscription string.
 * @return Node count (>= 0), or -1 on invalid input.
 */
int32_t vproxy_parse_nodes_count(const char* content);

/**
 * @brief Compiles a node and options into the target kernel's configuration format.
 * @param kernel_kind Target kernel: "mihomo", "sing-box", "vp-core", "xray", "shoes", "meow".
 * @param node_json JSON representation of a Node object.
 * @param options_json Optional JSON of MihomoConfigOptions (pass NULL for defaults).
 * @return Configuration text (YAML or JSON) or error `{"error":"..."}`. Must be freed with vproxy_free_string().
 */
char* vproxy_compile_config(const char* kernel_kind, const char* node_json, const char* options_json);

/**
 * @brief Tests TCP handshake latency to a given host and port in milliseconds.
 * @param host Null-terminated host name or IP address.
 * @param port Target TCP port.
 * @param timeout_ms Connection timeout in milliseconds.
 * @return Latency in ms (>= 1), or -1 on failure/timeout.
 */
int32_t vproxy_test_tcp_latency(const char* host, uint16_t port, uint32_t timeout_ms);

/* ── Backward-compatible Aliases ──────────────────────────────── */

const char* vpn_version(void);
void vpn_free_string(char* ptr);
char* vpn_parse_subscription(const char* content);
int32_t vpn_parse_nodes_count(const char* content);
char* vpn_compile_config(const char* kernel_kind, const char* node_json, const char* options_json);
int32_t vpn_test_tcp_latency(const char* host, uint16_t port, uint32_t timeout_ms);

#ifdef __cplusplus
}
#endif

#endif /* VPROXY_H */
