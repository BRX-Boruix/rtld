/* rtld 验收夹具之三（阶段 5 / 3P5-2）：**真正调用共享库**的动态程序。
 *
 * `printf` 来自 `libboruixc.so.1`——链接期留下 `DT_NEEDED: libboruixc.so.1`，调用点经 PLT
 * 走 `R_X86_64_JUMP_SLOT`。rtld 必须：递归装载该 `.so` → 解析跨模块符号 → 填 GOT。
 *
 * 返回 1 表示整条链打通（输出本身也由 `.so` 里的 printf 产生，是第二重证据）。
 */
extern int printf(const char *fmt, ...);

int check(void) {
    printf("hello from a dynamically linked program\n");
    return 1;
}