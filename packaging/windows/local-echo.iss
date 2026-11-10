#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{140AB8F3-F901-46F3-AA78-68A210345FB5}
AppName=Local Echo
AppVersion={#AppVersion}
AppPublisher=Local Echo
DefaultDirName={autopf}\Local Echo
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
SetupIconFile=..\..\assets\app.ico
UninstallDisplayIcon={app}\local-echo.exe
CloseApplications=force
RestartApplications=no
OutputDir=..\..\target\installer
OutputBaseFilename=local-echo-{#AppVersion}-windows-x86_64-setup

[Languages]
Name: "chs"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"

[Files]
Source: "..\..\target\release\local-echo.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Local Echo"; Filename: "{app}\local-echo.exe"
Name: "{autodesktop}\Local Echo"; Filename: "{app}\local-echo.exe"; Tasks: desktopicon

[Run]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Local Echo"""; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""Local Echo"" dir=in action=allow protocol=TCP localport=47210 program=""{app}\local-echo.exe"" profile=any"; Flags: runhidden
Filename: "{app}\local-echo.exe"; Description: "启动 Local Echo"; Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallRun]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Local Echo"""; Flags: runhidden; RunOnceId: "DelFwRule"
Filename: "{sys}\reg.exe"; Parameters: "delete ""HKCU\Software\Microsoft\Windows\CurrentVersion\Run"" /v ""Local Echo"" /f"; Flags: runhidden; RunOnceId: "DelAutostart"
