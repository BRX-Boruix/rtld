/* rtld 验收夹具（阶段 5 / 3P5-1）：一个**必须被重定位**才能正确运行的 PIE。
 *
 * `fp` 是全局函数指针——在 PIE 里它的初值需要 `R_X86_64_RELATIVE` 重定位（加装载基址）。
 * 若 rtld 不做重定位，`fp` 就是「未加基址的 vaddr」，调用它必然跑飞。
 *
 * rtld 按符号名找到 `check` 并调用：**返回 1 表示重定位生效**。
 *
 * 构建见同目录 build_pie.py（需要 `--export-dynamic`，否则 `check` 不在 `.dynsym` 里——
 * 实测过：PIE 的 `.dynsym` 默认只含动态链接真正需要的符号）。
 */
static int target(void) { return 42; }

int (*fp)(void) = target; /* ← 重定位点 */

int check(void) { return fp() == 42 ? 1 : 0; }