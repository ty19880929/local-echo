#!/usr/bin/env bash
# 在 Linux 上交叉编译 Windows 测试版（x86_64-pc-windows-gnu），见 docs/plan-v0.2.md 5.1。
# 正式安装包仍由 CI 在 Windows 上用 MSVC 构建，这里只用于测试。
#
# 依赖：rustup target add x86_64-pc-windows-gnu；mingw-w64（x86_64-w64-mingw32-gcc / windres）。
#
# 用法：
#   scripts/build-windows.sh           构建 release 版 local-echo.exe
#   scripts/build-windows.sh --tests   只编译单元测试程序（不运行），拷到测试机上执行
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=x86_64-pc-windows-gnu
SHADERS=packaging/windows/cross/shaders_bytes.rs

if [[ "${1:-}" == "--tests" ]]; then
    ARGS=(test --no-run --target "$TARGET")
else
    ARGS=(build --release --target "$TARGET")
fi

# 列出本次构建中 gpui-pre-windows 的 OUT_DIR（build script 已执行过时 cargo 也会报告）。
out_dirs() {
    python3 -c '
import json, sys
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    if msg.get("reason") == "build-script-executed" and "gpui-pre-windows" in msg.get("package_id", ""):
        print(msg["out_dir"])
'
}

# gpui-pre-windows 的 build.rs 只在 Windows 编译机上调用 fxc 编译 HLSL 着色器；交叉编译时跳过，
# release 构建会在 include!(OUT_DIR/shaders_bytes.rs) 处报错。先跑一遍拿到 OUT_DIR（这一遍
# 编译失败属正常），放入预编译的着色器，再正式构建。debug 构建在运行时编译着色器，不需要这个文件。
while read -r dir; do
    [[ -n "$dir" ]] && cp "$SHADERS" "$dir/shaders_bytes.rs"
done < <(cargo "${ARGS[@]}" --message-format=json 2>/dev/null | out_dirs || true)

cargo "${ARGS[@]}"

if [[ "${1:-}" == "--tests" ]]; then
    cargo "${ARGS[@]}" --message-format=json 2>/dev/null | python3 -c '
import json, sys
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    if msg.get("reason") == "compiler-artifact" and msg.get("profile", {}).get("test") and msg.get("executable"):
        print("单元测试程序：" + msg["executable"])
'
else
    echo "程序：target/$TARGET/release/local-echo.exe"
fi
