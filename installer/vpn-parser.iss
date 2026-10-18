#define MyAppName "VPN Parser"
; 版本号唯一真源 = Cargo.toml 的 version。build.rs 每次编译生成 target\cargo-version.iss
; （里面就一行 `#define MyAppVersion "x.y.z"`），这里 include 进来 —— 安装包文件名、
; "关于"里的版本、卸载列表里的版本从此不会再各说各话（以前这里硬写 "1.0"，
; 而 Cargo.toml 是 1.0.0、tag 是 v1.0.0）。
#if !FileExists("..\\target\\cargo-version.iss")
  #error 找不到 target\cargo-version.iss —— 先跑一次 cargo build --release，再编安装包
#endif
#include "..\target\cargo-version.iss"
#define MyAppPublisher "VPN Parser"
#define MyAppExeName "vpn-parser.exe"

[Setup]
AppId={{A660CFA2-3897-4D78-9C52-47D90B9B27F7}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\VPN Parser
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
UsedUserAreasWarning=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=vpn-parser-{#MyAppVersion}-windows-x64-setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\app-icon.ico
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\{#MyAppExeName}
; 这两个只收纯数字 x.x.x.x（预发布版号带 -beta.N 会让 ISCC 报错），所以用 build.rs
; 另算出来的 MyAppFileVersion；上面 AppVersion 和文件名仍用完整的 MyAppVersion。
VersionInfoVersion={#MyAppFileVersion}
VersionInfoProductName={#MyAppName}
VersionInfoProductVersion={#MyAppFileVersion}

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加任务:"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\data\kernels\mihomo\geosite.dat"; DestDir: "{app}\data\kernels\mihomo"; Flags: ignoreversion
Source: "..\data\kernels\mihomo\geoip.dat"; DestDir: "{app}\data\kernels\mihomo"; Flags: ignoreversion
Source: "..\data\kernels\mihomo\mihomo.exe"; DestDir: "{app}\data\kernels\mihomo"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "启动 {#MyAppName}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent

[Code]
function InitializeSetup(): Boolean;
var
  ResultCode: Integer;
begin
  // 不能带 /T：从「关于 → 安装」进来的时候，安装向导是本程序的子进程，
  // 带 /T 会连自己这棵树一起杀掉 —— 现象就是"点了安装，什么都没弹出来"
  // （2026-10-15 他报的）。按镜像名杀已经覆盖所有 vpn-parser.exe 实例（含提权那个
  // 子实例）；留下的 mihomo.exe 由新程序启动时"清理残留内核释放端口"那条路收掉。
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM vpn-parser.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := True;
end;
