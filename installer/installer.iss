#define AppName "Resenha Client"
#define AppVersion "0.1.0"


[Setup]

AppId={{RESENHA-CLIENT}}

AppName={#AppName}

AppVersion={#AppVersion}

DefaultDirName={autopf}\Resenha Client

DefaultGroupName=Resenha Client

OutputDir=Output

OutputBaseFilename=Resenha-Setup

Compression=lzma

SolidCompression=yes

WizardStyle=modern


[Files]

Source: "Build\resenha-client.exe"; DestDir: "{app}"; Flags: ignoreversion


[Icons]

Name: "{group}\Resenha Client"; Filename: "{app}\resenha-client.exe"

Name: "{autodesktop}\Resenha Client"; Filename: "{app}\resenha-client.exe"


[Run]

Filename: "{app}\resenha-client.exe"; Description: "Abrir Resenha Client"; Flags: nowait postinstall skipifsilent