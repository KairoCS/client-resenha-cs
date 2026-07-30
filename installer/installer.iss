#define AppName "KAIRO"
#define AppVersion "0.2.1"


[Setup]

AppId={{KAIRO-CLIENT}}

AppName={#AppName}

AppVersion={#AppVersion}

AppPublisher="KAIRO"

SetupIconFile=assets\icon.ico

DefaultDirName={autopf}\KAIRO

DefaultGroupName=KAIRO

OutputDir=Output

OutputBaseFilename=KAIRO-Setup

Compression=lzma

SolidCompression=yes

WizardStyle=modern


[Files]

Source: "Build\resenha-client.exe"; DestDir: "{app}"; Flags: ignoreversion


[Icons]

Name: "{group}\KAIRO"; Filename: "{app}\resenha-client.exe"

Name: "{autodesktop}\KAIRO"; Filename: "{app}\resenha-client.exe"


[Run]

Filename: "{app}\resenha-client.exe"; \
Description: "Abrir KAIRO"; \
Flags: nowait postinstall skipifsilent