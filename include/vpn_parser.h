/**
 * @file vpn_parser.h
 * @brief C-ABI bindings for vpn-parser (Headless Core)
 * 
 * Cross-platform C bindings supporting Android JNI, iOS Swift/ObjC, 
 * HarmonyOS C/NAPI, Python ctypes, and Node.js FFI.
 */

#ifndef VPN_PARSER_H
#define VPN_PARSER_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Returns the version string of the vpn-parser library.
 * @return Statically allocated null-terminated string (do not free).
 */
const char* vpn_version(void);

/**
 * @brief Frees a string previously allocated and returned by vpn-parser FFI functions.
 * @param ptr Pointer to the string to free. Passing NULL is safe.
 */
void vpn_free_string(char* ptr);

/**
 * @brief Parses raw subscription text or node URIs into a JSON array string.
 * @param content Null-terminated string containing base64 subscription, Clash YAML, Sing-box JSON, or URIs.
 * @return JSON array of parsed nodes `[{"id":"...", ...}]` or error `{"error":"..."}`. Must be freed with vpn_free_string().
 */
char* vpn_parse_subscription(const char* content);

/**
 * @brief Returns the number of valid proxy nodes in the subscription.
 * @param content Null-terminated subscription string.
 * @return Node count (>= 0), or -1 on invalid input.
 */
int32_t vpn_parse_nodes_count(const char* content);

/**
 * @brief Compiles a node and options into the target kernel's configuration format.
 * @param kernel_kind Target kernel: "mihomo", "sing-box", "vp-core", "xray", "shoes", "meow".
 * @param node_json JSON representation of a Node object.
 * @param options_json Optional JSON of MihomoConfigOptions (pass NULL for defaults).
 * @return Configuration text (YAML or JSON) or error `{"error":"..."}`. Must be freed with vpn_free_string().
 */
char* vpn_compile_config(const char* kernel_kind, const char* node_json, const char* options_json);

/**
 * @brief Fast synchronous TCP latency probe in milliseconds.
 * @param host Server hostname or IP address.
 * @param port Port number.
 * @param timeout_ms Timeout in milliseconds.
 * @return Round-trip latency in ms (>= 0), or -1 on timeout/failure.
 */
int32_t vpn_test_tcp_latency(const char* host, uint16_t port, uint32_t timeout_ms);

#ifdef __cplusplus
}
#endif

#endif /* VPN_PARSER_H */
