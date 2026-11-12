# 在 Windows 上用系统自带的 d3dcompiler_47.dll（fxc.exe 内部用的就是它）预编译 gpui 的 HLSL 着色器，
# 生成与 gpui-pre-windows 的 build.rs 格式相同的 shaders_bytes.rs，供 Linux 交叉编译使用。
# 不需要安装 Windows SDK 或任何开发工具。
#
# 用法（在 Windows 上，PowerShell）：
#   1. 把 gpui-pre-windows-<版本>/src/shaders.hlsl 拷到某个临时目录
#   2. powershell -ExecutionPolicy Bypass -File compile-shaders.ps1 -Dir <临时目录>
#   3. 把生成的 <临时目录>\shaders_bytes.rs 拷回本目录，删除临时目录
#
# 编译参数与 build.rs 一致：入口 <模块>_vertex / <模块>_fragment，vs_4_1 / ps_4_1，/O3。
# gpui 固定在 0.3.12，着色器不变就不用重新生成。
param([Parameter(Mandatory = $true)][string]$Dir)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
[ComImport, Guid("8BA5FB08-5195-40e2-AC58-0D989C3A0102"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface ID3DBlob { [PreserveSig] IntPtr GetBufferPointer(); [PreserveSig] UIntPtr GetBufferSize(); }
public static class Fxc {
  [DllImport("d3dcompiler_47.dll")]
  static extern int D3DCompileFromFile([MarshalAs(UnmanagedType.LPWStr)] string file, IntPtr defines, IntPtr include,
    [MarshalAs(UnmanagedType.LPStr)] string entry, [MarshalAs(UnmanagedType.LPStr)] string target,
    uint flags1, uint flags2, out ID3DBlob code, out ID3DBlob errors);
  // flags1 = D3DCOMPILE_OPTIMIZATION_LEVEL3 (1 << 15)，即 fxc 的 /O3；include = D3D_COMPILE_STANDARD_FILE_INCLUDE。
  public static byte[] Compile(string file, string entry, string target) {
    ID3DBlob code, err;
    int hr = D3DCompileFromFile(file, IntPtr.Zero, new IntPtr(1), entry, target, 1u << 15, 0, out code, out err);
    if (hr < 0) { string m = err != null ? Marshal.PtrToStringAnsi(err.GetBufferPointer()) : ""; throw new Exception(entry + " 0x" + hr.ToString("X8") + " " + m); }
    int n = (int)code.GetBufferSize().ToUInt64(); byte[] b = new byte[n]; Marshal.Copy(code.GetBufferPointer(), b, 0, n); return b;
  }
}
"@
$src = Join-Path $Dir 'shaders.hlsl'
$out = New-Object System.Text.StringBuilder
foreach ($m in 'quad','shadow','path_rasterization','path_sprite','underline','monochrome_sprite','subpixel_sprite','polychrome_sprite') {
  foreach ($t in @(@('vertex','vs_4_1','VERTEX'), @('fragment','ps_4_1','FRAGMENT'))) {
    $bytes = [Fxc]::Compile($src, "$($m)_$($t[0])", $t[1])
    [void]$out.Append("const $($m.ToUpper())_$($t[2])_BYTES: &[u8] = &[" + ($bytes -join ', ') + "];`n")
    "$($m)_$($t[0]): $($bytes.Length) bytes"
  }
}
[IO.File]::WriteAllText((Join-Path $Dir 'shaders_bytes.rs'), $out.ToString())
"d3dcompiler: " + (Get-Item "$env:WINDIR\System32\d3dcompiler_47.dll").VersionInfo.FileVersion
