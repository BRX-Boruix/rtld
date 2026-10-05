/* 带 TLS 的共享库：用于验证 ADR-053 D4——`dlopen` 必须**如实拒绝**它。 */
__thread int tls_var = 7;
int tls_get(void) { return tls_var; }