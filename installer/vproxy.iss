#define MyAppName "VProxy"
; 版本号唯一真源 = Cargo.toml 的 version。build.rs 每次编译生成 target\cargo-version.iss
; （里面就一行 `#define MyAppVersion "x.y.z"`），这里 include 进来 —— 安装包文件名、
; "关于"里的版本、卸载列表里的版本从此不会再各说各话。
#if !FileExists("..\\target\\cargo-version.iss")
  #error 找不到 target\cargo-version.iss —— 先跑一次 cargo build --release，再编安装包
#endif
#include "..\target\cargo-version.iss"
#define MyAppPublisher "VProxy"
#define MyAppExeName "vproxy.exe"

[Setup]
AppId={{A660CFA2-3897-4D78-9C52-47D90B9B27F7}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\VProxy
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
UsedUserAreasWarning=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=vproxy-{#MyAppVersion}-windows-x64-setup
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

[InstallDelete]
Type: files; Name: "{app}\vpn-parser.exe"
Type: files; Name: "{autodesktop}\VPN Parser.lnk"
Type: files; Name: "{group}\VPN Parser.lnk"

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "启动 {#MyAppName}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent runascurrentuser

[Code]
function InitializeSetup(): Boolean;
var
  ResultCode: Integer;
begin
  // 1. 按镜像名彻底关闭所有旧版与新版主进程
  // 注意：绝对不能带 /T 参数！老程序点击「更新/安装」时安装包作为子进程运行，
  // 若带 /T 会将整个进程树一并杀死（导致向导自身闪退，表现为"点击安装无反应/弹不出向导"）
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM vproxy.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM vpn-parser.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);

  // 2. 防御性终止可能残留的后台代理内核进程，确保 data/kernels 目录下的文件句柄全部释放
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM mihomo.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM sing-box.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM xray.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM meow.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM shoes.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM vp-core.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := True;
end;

procedure InitializeWizard();
begin
  // 将安装向导窗口显式激活并置于前台，避免被老程序退出后前置的浏览器或其他全屏窗口遮挡
  WizardForm.BringToFront();
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  OldAppDir: String;
  OldDataDir: String;
  NewDataDir: String;
  ResultCode: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    NewDataDir := ExpandConstant('{app}\data');

    // 智能获取历史安装目录：优先从历史卸载注册表读取真实路径（覆盖自定义安装盘符情况）
    OldAppDir := '';
    if not RegQueryStringValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{A660CFA2-3897-4D78-9C52-47D90B9B27F7}_is1', 'InstallLocation', OldAppDir) then
    begin
      RegQueryStringValue(HKEY_LOCAL_MACHINE, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{A660CFA2-3897-4D78-9C52-47D90B9B27F7}_is1', 'InstallLocation', OldAppDir);
    end;

    if (OldAppDir <> '') and DirExists(OldAppDir + '\data') then
    begin
      OldDataDir := OldAppDir + '\data';
    end
    else
    begin
      OldDataDir := ExpandConstant('{localappdata}\Programs\VPN Parser\data');
    end;

    // 若旧数据目录存在、路径与新目录不同且新应用尚未生成 data 目录，自动全量平滑迁移（/E /I /Y /C /H /R /K）
    if DirExists(OldDataDir) and (CompareText(OldDataDir, NewDataDir) <> 0) and not DirExists(NewDataDir) then
    begin
      Exec(ExpandConstant('{sys}\cmd.exe'), '/c xcopy /E /I /Y /C /H /R /K "' + OldDataDir + '" "' + NewDataDir + '"', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    end;

    // 清理历史残留：旧桌面快捷方式、旧开始菜单项、同目录下的旧可执行文件
    DeleteFile(ExpandConstant('{autodesktop}\VPN Parser.lnk'));
    DeleteFile(ExpandConstant('{group}\VPN Parser.lnk'));
    DeleteFile(ExpandConstant('{app}\vpn-parser.exe'));
    if (OldAppDir <> '') and (CompareText(OldAppDir, ExpandConstant('{app}')) <> 0) then
    begin
      DeleteFile(OldAppDir + '\vpn-parser.exe');
      DeleteFile(OldAppDir + '\unins000.exe');
      DeleteFile(OldAppDir + '\unins000.dat');
    end;

    // 清理旧注册表开机自启项 (VPN Parser -> VProxy 由程序启动自动接管)
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'VPN Parser');
  end;
end;
