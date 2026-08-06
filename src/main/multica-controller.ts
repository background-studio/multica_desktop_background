import { execFile } from "node:child_process";
import { promises as fs } from "node:fs";
import net from "node:net";
import path from "node:path";
import { promisify } from "node:util";
import { RuntimeStatus } from "../shared/contracts.js";
import { InjectorEngine, readBrowserIdentity } from "./injector.js";

const execFileAsync = promisify(execFile);

interface MulticaInstall {
  packageRoot: string;
  executable: string;
  version: string;
  packageFullName: string;
  packageFamilyName: string;
  applicationId: string;
  appUserModelId: string;
}

interface RuntimeState {
  schemaVersion: 1;
  port: number;
  browserId: string;
  packageFullName: string;
  executable: string;
  createdAt: string;
}

const DISCOVER_SCRIPT = String.raw`
$ErrorActionPreference = 'Stop'
$packages = @(Get-AppxPackage -Name 'Notion.Desktop' | Sort-Object Version -Descending)
foreach ($package in $packages) {
  if ("$($package.SignatureKind)" -ine 'Store' -or [bool]$package.IsDevelopmentMode) { continue }
  $exe = Join-Path "$($package.InstallLocation)" 'Multica.exe'
  if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { continue }
  $manifest = Get-AppxPackageManifest -Package $package
  $apps = @($manifest.Package.Applications.Application | Where-Object {
    "$($_.Executable)".Replace('/', '\') -ieq 'Multica.exe'
  })
  if ($apps.Count -ne 1) { continue }
  $id = "$($apps[0].Id)"
  $family = "$($package.PackageFamilyName)"
  if ($family -cnotmatch '^[A-Za-z0-9._-]{1,128}$' -or $id -cnotmatch '^[A-Za-z0-9._-]{1,64}$') { continue }
  [pscustomobject]@{
    packageRoot = "$($package.InstallLocation)"
    executable = $exe
    version = "$($package.Version)"
    packageFullName = "$($package.PackageFullName)"
    packageFamilyName = $family
    applicationId = $id
    appUserModelId = "$family!$id"
  } | ConvertTo-Json -Compress
  exit 0
}
throw '未找到经过验证的官方 Multica.Desktop Store 应用。'
`;

function quotePowerShell(value: string) {
  return `'${value.replace(/'/g, "''")}'`;
}

function encodedPowerShell(script: string) {
  return Buffer.from(script, "utf16le").toString("base64");
}

async function runPowerShell(script: string, timeout = 30_000) {
  const { stdout } = await execFileAsync("powershell.exe", [
    "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
    "-EncodedCommand", encodedPowerShell(script),
  ], { timeout, windowsHide: true, maxBuffer: 4 * 1024 * 1024 });
  return stdout.trim();
}

async function discoverNotion(): Promise<MulticaInstall> {
  const raw = await runPowerShell(DISCOVER_SCRIPT);
  const result = JSON.parse(raw) as MulticaInstall;
  if (!result.appUserModelId.match(/^[A-Za-z0-9._-]{1,128}![A-Za-z0-9._-]{1,64}$/) ||
    !path.resolve(result.executable).toLowerCase().startsWith(`${path.resolve(result.packageRoot).toLowerCase()}${path.sep}`)) {
    throw new Error("Multica 安装身份校验失败。");
  }
  return result;
}

async function processIdsFor(install: MulticaInstall) {
  const script = String.raw`
$target = ${quotePowerShell(path.resolve(install.executable))}
$ids = @(Get-CimInstance Win32_Process -Filter "Name='Multica.exe'" | Where-Object {
  $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).Equals($target, [StringComparison]::OrdinalIgnoreCase)
} | ForEach-Object { [int]$_.ProcessId })
@($ids) | ConvertTo-Json -Compress
`;
  const raw = await runPowerShell(script);
  const value = raw ? JSON.parse(raw) : [];
  return Array.isArray(value) ? value.map(Number).filter(Number.isInteger) : [Number(value)].filter(Number.isInteger);
}

async function stopVerifiedNotion(install: MulticaInstall) {
  const executable = path.resolve(install.executable);
  const script = String.raw`
$ErrorActionPreference = 'Stop'
$target = ${quotePowerShell(executable)}
$processes = @(Get-CimInstance Win32_Process -Filter "Name='Multica.exe'" | Where-Object {
  $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).Equals($target, [StringComparison]::OrdinalIgnoreCase)
})
foreach ($item in $processes) { Stop-Process -Id ([int]$item.ProcessId) -Force -ErrorAction SilentlyContinue }
`;
  await runPowerShell(script);
  const deadline = Date.now() + 15_000;
  while ((await processIdsFor(install)).length > 0) {
    if (Date.now() >= deadline) throw new Error("Multica 未能在 15 秒内完全退出。");
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
}

async function launchNotion(install: MulticaInstall, args: string[]) {
  if (!install.appUserModelId.match(/^[A-Za-z0-9._-]{1,128}![A-Za-z0-9._-]{1,64}$/)) {
    throw new Error("Notion AppUserModelId 无效。");
  }
  const argumentLine = args.map((entry) => `"${entry.replace(/"/g, '\\"')}"`).join(" ");
  const script = String.raw`
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace MulticaBackgroundStudio {
  [ComImport, Guid("2e941141-7f97-4756-ba1d-9decde894a3d"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IApplicationActivationManager {
    [PreserveSig] int ActivateApplication(
      [MarshalAs(UnmanagedType.LPWStr)] string appUserModelId,
      [MarshalAs(UnmanagedType.LPWStr)] string arguments,
      uint options,
      out uint processId);
  }
  [ComImport, Guid("45ba127d-10a8-46ea-8ab7-56ea9078943c")]
  class ApplicationActivationManager {}
  public static class Launcher {
    public static uint Launch(string id, string arguments) {
      var manager = (IApplicationActivationManager)new ApplicationActivationManager();
      try {
        uint processId;
        int result = manager.ActivateApplication(id, arguments ?? "", 0, out processId);
        Marshal.ThrowExceptionForHR(result);
        return processId;
      } finally {
        if (Marshal.IsComObject(manager)) Marshal.FinalReleaseComObject(manager);
      }
    }
  }
}
'@
$launchedProcessId = [MulticaBackgroundStudio.Launcher]::Launch(${quotePowerShell(install.appUserModelId)}, ${quotePowerShell(argumentLine)})
if ($launchedProcessId -le 0) { throw 'Windows 未返回 Multica 进程 ID。' }
$launchedProcessId
`;
  await runPowerShell(script);
}

async function portAvailable(port: number) {
  return new Promise<boolean>((resolve) => {
    const server = net.createServer();
    server.once("error", () => resolve(false));
    server.listen(port, "127.0.0.1", () => server.close(() => resolve(true)));
  });
}

async function selectPort(preferred = 9227) {
  for (let port = preferred; port <= Math.min(65535, preferred + 100); port += 1) {
    if (await portAvailable(port)) return port;
  }
  throw new Error("无法为 Multica 分配本机调试端口。");
}

export class MulticaController {
  readonly statePath: string;
  #engine: InjectorEngine | null = null;
  #state: RuntimeState | null = null;
  #status: RuntimeStatus = { phase: "idle", message: "尚未连接 Multica", activeTargets: 0 };

  constructor(readonly dataDirectory: string, readonly onStatus: (status: RuntimeStatus) => void) {
    this.statePath = path.join(dataDirectory, "runtime.json");
  }

  get status() { return structuredClone(this.#status); }

  async initialize() {
    try {
      const state = JSON.parse(await fs.readFile(this.statePath, "utf8")) as RuntimeState;
      if (state.schemaVersion === 1 && Number.isInteger(state.port) && state.browserId) this.#state = state;
    } catch {}
    return this.status;
  }

  #setStatus(patch: Partial<RuntimeStatus>) {
    this.#status = { ...this.#status, ...patch };
    this.onStatus(this.status);
  }

  async #writeState(state: RuntimeState | null) {
    this.#state = state;
    if (!state) return fs.rm(this.statePath, { force: true });
    const temporary = `${this.statePath}.${process.pid}.tmp`;
    await fs.writeFile(temporary, `${JSON.stringify(state, null, 2)}\n`, "utf8");
    await fs.rm(this.statePath, { force: true });
    await fs.rename(temporary, this.statePath);
  }

  async #tryAttachSaved(install: MulticaInstall) {
    const state = this.#state;
    if (!state || state.packageFullName !== install.packageFullName ||
      path.resolve(state.executable).toLowerCase() !== path.resolve(install.executable).toLowerCase()) return false;
    try {
      const browserId = await readBrowserIdentity(state.port);
      if (browserId !== state.browserId) return false;
      this.#engine = new InjectorEngine(state.port, state.browserId, (activeTargets) => {
        this.#setStatus({ activeTargets });
      });
      return true;
    } catch {
      return false;
    }
  }

  async apply(payload: string, revision: string, restartExisting = false) {
    this.#setStatus({ phase: "starting", message: "正在连接 Multica", lastError: undefined });
    try {
      const install = await discoverNotion();
      if (this.#engine) {
        await this.#engine.update(payload, revision);
        this.#setStatus({ phase: "active", message: "背景已实时应用", multicaVersion: install.version });
        return this.status;
      }
      if (await this.#tryAttachSaved(install)) {
        await this.#engine!.start(payload, revision);
        this.#setStatus({ phase: "active", message: "已重新连接背景会话", multicaVersion: install.version });
        return this.status;
      }

      const running = await processIdsFor(install);
      if (running.length > 0 && !restartExisting) {
        const error = new Error("Multica 需要重启一次以启用背景。");
        (error as Error & { code?: string }).code = "RESTART_REQUIRED";
        throw error;
      }
      if (running.length > 0) await stopVerifiedNotion(install);
      const port = await selectPort();
      await launchNotion(install, [
        "--remote-debugging-address=127.0.0.1",
        `--remote-debugging-port=${port}`,
      ]);
      const deadline = Date.now() + 45_000;
      let browserId = "";
      while (!browserId) {
        try { browserId = await readBrowserIdentity(port); } catch {}
        if (browserId) break;
        if (Date.now() >= deadline) throw new Error("Multica 未能在 45 秒内打开安全的本机调试端口。");
        await new Promise((resolve) => setTimeout(resolve, 400));
      }
      const state: RuntimeState = {
        schemaVersion: 1,
        port,
        browserId,
        packageFullName: install.packageFullName,
        executable: install.executable,
        createdAt: new Date().toISOString(),
      };
      await this.#writeState(state);
      this.#engine = new InjectorEngine(port, browserId, (activeTargets) => {
        this.#setStatus({ activeTargets });
      });
      await this.#engine.start(payload, revision);
      this.#setStatus({ phase: "active", message: "背景已应用", multicaVersion: install.version });
      return this.status;
    } catch (error) {
      const code = (error as Error & { code?: string }).code;
      this.#setStatus({
        phase: code === "RESTART_REQUIRED" ? "idle" : "error",
        message: (error as Error).message,
        lastError: (error as Error).message,
      });
      throw error;
    }
  }

  async pause() {
    if (this.#engine) await this.#engine.pause();
    this.#setStatus({ phase: "paused", message: "背景已暂停", lastError: undefined });
    return this.status;
  }

  async restore() {
    this.#setStatus({ phase: "restoring", message: "正在恢复官方外观", lastError: undefined });
    try {
      await this.#engine?.stop();
      this.#engine = null;
      const install = await discoverNotion();
      const running = await processIdsFor(install);
      if (running.length > 0) {
        await stopVerifiedNotion(install);
        await launchNotion(install, []);
      }
      await this.#writeState(null);
      this.#setStatus({
        phase: "idle",
        message: "已恢复官方外观",
        multicaVersion: install.version,
        activeTargets: 0,
      });
      return this.status;
    } catch (error) {
      this.#setStatus({ phase: "error", message: (error as Error).message, lastError: (error as Error).message });
      throw error;
    }
  }
}
