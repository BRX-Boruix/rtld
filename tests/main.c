/* rtld 验收夹具之二：引用共享库的 PIE（阶段 5 / 3P5-1 的真正验收对象）。
 *
 * `add` 来自 libadd.so —— 链接期留下 `DT_NEEDED: libadd.so`，调用点经 PLT 走
 * `R_X86_64_JUMP_SLOT` 重定位（GOT 槽由 rtld 填成 add 的实际地址）。
 *
 * rtld 装载 main.elf + 递归装载 libadd.so + 做重定位，然后调用 check：
 * **返回 1 表示跨模块调用真的打通了**。
 */
extern int add(int a, int b);

int check(void) { return add(20, 22) == 42 ? 1 : 0; }