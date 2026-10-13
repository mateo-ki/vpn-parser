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
VersionInfoVersion={#MyAppVersion}
VersionInfoProductName={#MyAppName}
VersionInfoProductVersion={#MyAppVersion}

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加任务:"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

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
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM vpn-parser.exe /T', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := True;
end;
