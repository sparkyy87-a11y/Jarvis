; Mira – Installer für Windows (Inno Setup 6)
; Gebaut von .github/workflows/release.yml:
;   iscc /DAppVersion=0.1.0 /DBinDir=..\..\target\release installer\windows\mira.iss
;
; Installiert pro Benutzer (keine Adminrechte). Nur die optionale
; Firewall-Freigabe fragt einmal nach Administratorrechten.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef BinDir
  #define BinDir "..\..\target\release"
#endif

[Setup]
AppId={{6E1C2B8A-4F7D-4C3B-9A51-3D1E7A2F9B10}
AppName=Mira
AppVersion={#AppVersion}
AppPublisher=Mira
DefaultDirName={localappdata}\Programs\Mira
DefaultGroupName=Mira
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputBaseFilename=Mira-Setup-{#AppVersion}
OutputDir=..\..\dist
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayName=Mira
ChangesEnvironment=yes
CloseApplications=no

[Languages]
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "sprache"; Description: "Spracheingabe „Mira, …“ (lädt einmalig ~190 MB Sprachmodell)"
Name: "hud"; Description: "HUD einblenden, wenn du mit Mira sprichst"
Name: "ollama"; Description: "Kostenloses lokales Sprachmodell (Ollama) – nur auf einem Gerät nötig, z. B. dem PC"; Flags: unchecked
Name: "ollama\klein"; Description: "Kleines Modell (qwen2.5:3b, ~2 GB) – läuft auf jedem PC"; Flags: exclusive
Name: "ollama\gross"; Description: "Großes Modell (qwen2.5:7b, ~5 GB) – mit Grafikkarte"; Flags: exclusive unchecked
Name: "desktopicon"; Description: "Verknüpfung „Mira-Einstellungen“ auf dem Desktop"
Name: "firewall"; Description: "Firewall nur für das Heimnetz (private Netzwerke) freigeben – fragt einmal nach Administratorrechten"

[Files]
Source: "{#BinDir}\mirad.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\mira.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\mira-listen.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\mira-hud.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\mira-hintergrund.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\mira-einstellungen.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Mira-Einstellungen"; Filename: "{app}\mira-einstellungen.exe"
Name: "{autodesktop}\Mira-Einstellungen"; Filename: "{app}\mira-einstellungen.exe"; Tasks: desktopicon
Name: "{group}\Gerät koppeln (QR-Code)"; Filename: "{cmd}"; Parameters: "/k ""{app}\mirad.exe"" pair"; WorkingDir: "{app}"
Name: "{group}\Gekoppelte Geräte"; Filename: "{cmd}"; Parameters: "/k ""{app}\mirad.exe"" geraete"; WorkingDir: "{app}"
Name: "{group}\Mira-Konsole"; Filename: "{app}\mira.exe"; WorkingDir: "{app}"
Name: "{group}\HUD-Vorschau"; Filename: "{app}\mira-hud.exe"; Parameters: "--demo"
Name: "{group}\Mira deinstallieren"; Filename: "{uninstallexe}"

[Registry]
; `mira` und `mirad` im Terminal verfügbar machen (nur für diesen Benutzer)
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath(ExpandConstant('{app}'))

[Run]
; 1. Einrichten (nur wenn es noch keine Konfiguration gibt – Updates behalten alles)
Filename: "{app}\mirad.exe"; Parameters: "init --name ""{code:DeviceName}"" {code:SetupFlags}"; StatusMsg: "Mira wird eingerichtet …"; Flags: runhidden waituntilterminated; Check: NeedsInit
Filename: "{app}\mirad.exe"; Parameters: "join ""{code:PairCode}"" --name ""{code:DeviceName}"" {code:SetupFlags}"; StatusMsg: "Kopplung wird vorbereitet …"; Flags: runhidden waituntilterminated; Check: NeedsJoin
; 2. Firewall nur für private Netzwerke (einmal Administratorrechte)
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""Mira"" dir=in action=allow program=""{app}\mirad.exe"" protocol=TCP localport=7879 profile=private enable=yes"; Verb: "runas"; StatusMsg: "Firewall wird für das Heimnetz freigegeben …"; Flags: shellexec runhidden waituntilterminated; Tasks: firewall
; 3. Kostenloses Sprachmodell
Filename: "winget.exe"; Parameters: "install -e --id Ollama.Ollama --silent --accept-source-agreements --accept-package-agreements"; StatusMsg: "Ollama wird installiert …"; Flags: runhidden waituntilterminated; Tasks: ollama
Filename: "{localappdata}\Programs\Ollama\ollama.exe"; Parameters: "pull qwen2.5:3b"; StatusMsg: "Sprachmodell wird geladen (ca. 2 GB) …"; Flags: waituntilterminated; Tasks: ollama\klein
Filename: "{localappdata}\Programs\Ollama\ollama.exe"; Parameters: "pull qwen2.5:7b"; StatusMsg: "Sprachmodell wird geladen (ca. 5 GB) …"; Flags: waituntilterminated; Tasks: ollama\gross
; 4. Whisper-Modell vorab laden
Filename: "{app}\mira-listen.exe"; Parameters: "--nur-modelle"; StatusMsg: "Spracherkennung wird geladen (ca. 190 MB) …"; Flags: waituntilterminated; Tasks: sprache
; 5. Starten und bei jeder Anmeldung starten
Filename: "{app}\mirad.exe"; Parameters: "autostart an"; StatusMsg: "Mira wird gestartet …"; Flags: runhidden waituntilterminated
; 6. Zum Schluss: QR-Code für das nächste Gerät anzeigen
Filename: "{app}\mira-einstellungen.exe"; Description: "Mira-Einstellungen öffnen (Geräte koppeln, Mikrofon, Sprachmodell …)"; Flags: postinstall nowait skipifsilent

[UninstallRun]
Filename: "{app}\mirad.exe"; Parameters: "autostart aus"; Flags: runhidden waituntilterminated; RunOnceId: "MiraStop"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Mira"""; Verb: "runas"; Flags: shellexec runhidden waituntilterminated skipifdoesntexist; RunOnceId: "MiraFirewall"

[Code]
var
  NamePage: TInputQueryWizardPage;
  ModePage: TInputOptionWizardPage;
  CodePage: TInputQueryWizardPage;

function ConfigFile: String;
begin
  Result := ExpandConstant('{userappdata}\mira\config.toml');
end;

function AlreadySetUp: Boolean;
begin
  Result := FileExists(ConfigFile);
end;

procedure InitializeWizard;
begin
  NamePage := CreateInputQueryPage(wpSelectTasks,
    'Name dieses Geräts', 'So sprichst du das Gerät an – z. B. „am PC“, „am Laptop“.',
    'Tipp: Nimm einen Namen, der das Wort enthält, das du sagen willst (Gaming-PC, Laptop, Büro-PC).');
  NamePage.Add('Name:', False);
  NamePage.Values[0] := GetComputerNameString;

  ModePage := CreateInputOptionPage(NamePage.ID,
    'Erstes Gerät oder koppeln?', 'Ist das dein erstes Mira-Gerät?',
    'Das erste Gerät wird dein HAUPTGERÄT: nur dort koppelst du neue Geräte, sperrst sie und vergibst Rechte. Installiere es also auf dem Gerät, das du selbst benutzt (z. B. deinem Laptop). Weitere Geräte koppelst du mit einem Einmal-Code vom Hauptgerät.',
    True, False);
  ModePage.Add('Das ist mein erstes Mira-Gerät – es wird das Hauptgerät');
  ModePage.Add('Mit einem vorhandenen Mira koppeln (Code einfügen)');
  ModePage.Values[0] := True;

  CodePage := CreateInputQueryPage(ModePage.ID,
    'Kopplungscode', 'Code vom Hauptgerät einfügen',
    'Am Hauptgerät: Startmenü › Mira › „Gerät koppeln“ und die Zeile mira://pair?… kopieren. Der Code gilt 10 Minuten.');
  CodePage.Add('Code (mira://pair?…):', False);
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := False;
  { Bei Updates nichts fragen – die Einrichtung bleibt erhalten. }
  if AlreadySetUp and ((PageID = NamePage.ID) or (PageID = ModePage.ID) or (PageID = CodePage.ID)) then
    Result := True
  else if PageID = CodePage.ID then
    Result := ModePage.Values[0];
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = NamePage.ID) and (Trim(NamePage.Values[0]) = '') then
  begin
    MsgBox('Bitte gib dem Gerät einen Namen.', mbError, MB_OK);
    Result := False;
  end;
  if (CurPageID = CodePage.ID) and (Pos('mira://pair?', Trim(CodePage.Values[0])) <> 1) then
  begin
    MsgBox('Das ist kein Kopplungscode. Er beginnt mit mira://pair?', mbError, MB_OK);
    Result := False;
  end;
end;

function Sanitize(S: String): String;
begin
  { Anführungszeichen würden die Kommandozeile aufbrechen. }
  StringChangeEx(S, '"', '', True);
  Result := Trim(S);
end;

function DeviceName(Param: String): String;
begin
  Result := Sanitize(NamePage.Values[0]);
end;

function PairCode(Param: String): String;
begin
  Result := Sanitize(CodePage.Values[0]);
end;

function SetupFlags(Param: String): String;
begin
  Result := '';
  if not WizardIsTaskSelected('sprache') then Result := Result + ' --ohne-sprache';
  if not WizardIsTaskSelected('hud') then Result := Result + ' --ohne-hud';
  if WizardIsTaskSelected('ollama\gross') then Result := Result + ' --sprachmodell qwen2.5:7b';
  if not WizardIsTaskSelected('ollama') then Result := Result + ' --ohne-sprachmodell';
end;

function NeedsInit: Boolean;
begin
  Result := (not AlreadySetUp) and ModePage.Values[0];
end;

function NeedsJoin: Boolean;
begin
  Result := (not AlreadySetUp) and ModePage.Values[1];
end;

function NeedsAddPath(Dir: String): Boolean;
var
  Path: String;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Path) then
    Result := True
  else
    Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') = 0;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  { Bei Updates laufendes Mira vorher anhalten, damit Dateien ersetzt werden können. }
  if (CurStep = ssInstall) and FileExists(ExpandConstant('{app}\mirad.exe')) then
    Exec(ExpandConstant('{app}\mirad.exe'), 'autostart aus', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;
