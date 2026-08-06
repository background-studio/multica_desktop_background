import { earlyPayloadFor, REMOVE_RENDERER_PAYLOAD } from "./payload.js";

const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "[::1]", "::1"]);
const ID_PATTERN = /^[A-Za-z0-9._-]{1,200}$/;

interface CdpTarget {
  id: string;
  type: string;
  url: string;
  webSocketDebuggerUrl: string;
}

interface CdpVersion {
  webSocketDebuggerUrl: string;
}

function validateWebSocketUrl(value: string, port: number, kind: "page" | "browser", id?: string) {
  const url = new URL(value);
  const expected = id ? `/devtools/${kind}/${id}` : `/devtools/${kind}/`;
  if (url.protocol !== "ws:" || !LOOPBACK_HOSTS.has(url.hostname) || Number(url.port) !== port ||
    url.username || url.password || url.search || url.hash ||
    (id ? url.pathname !== expected : !url.pathname.startsWith(expected))) {
    throw new Error("CDP WebSocket 地址未通过本机回环校验。");
  }
  return url.href;
}

async function fetchJson<T>(port: number, resource: string): Promise<T> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 3000);
  try {
    const response = await fetch(`http://127.0.0.1:${port}${resource}`, {
      signal: controller.signal,
      cache: "no-store",
    });
    if (!response.ok) throw new Error(`CDP 返回 HTTP ${response.status}`);
    return await response.json() as T;
  } finally {
    clearTimeout(timeout);
  }
}

export async function readBrowserIdentity(port: number) {
  const version = await fetchJson<CdpVersion>(port, "/json/version");
  const url = new URL(validateWebSocketUrl(version.webSocketDebuggerUrl, port, "browser"));
  const match = url.pathname.match(/^\/devtools\/browser\/([A-Za-z0-9._-]{1,200})$/);
  if (!match) throw new Error("CDP 浏览器身份无效。");
  return match[1];
}

async function listTargets(port: number, browserId: string) {
  const currentId = await readBrowserIdentity(port);
  if (currentId !== browserId) throw new Error("CDP 浏览器身份已变化，拒绝继续注入。");
  const targets = await fetchJson<CdpTarget[]>(port, "/json/list");
  return targets.filter((target) => {
    if (target.type !== "page" || !target.url.startsWith("app://") || !ID_PATTERN.test(target.id)) return false;
    try {
      validateWebSocketUrl(target.webSocketDebuggerUrl, port, "page", target.id);
      return true;
    } catch {
      return false;
    }
  });
}

interface PendingCall {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timeout: NodeJS.Timeout;
}

class CdpSession {
  readonly targetId: string;
  #socket: WebSocket;
  #nextId = 1;
  #pending = new Map<number, PendingCall>();
  #closed = false;

  constructor(target: CdpTarget, port: number) {
    this.targetId = target.id;
    this.#socket = new WebSocket(validateWebSocketUrl(target.webSocketDebuggerUrl, port, "page", target.id));
  }

  get closed() { return this.#closed; }

  async open() {
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error("CDP 连接超时。")), 5000);
      this.#socket.addEventListener("open", () => { clearTimeout(timeout); resolve(); }, { once: true });
      this.#socket.addEventListener("error", () => { clearTimeout(timeout); reject(new Error("CDP 连接失败。")); }, { once: true });
    });
    this.#socket.addEventListener("message", (event) => this.#onMessage(String(event.data)));
    this.#socket.addEventListener("close", () => this.close());
    this.#socket.addEventListener("error", () => this.close());
    await Promise.all([this.send("Runtime.enable"), this.send("Page.enable")]);
    return this;
  }

  #onMessage(raw: string) {
    let message: { id?: number; result?: unknown; error?: { message: string; code: number } };
    try { message = JSON.parse(raw); } catch { return; }
    if (!message.id) return;
    const pending = this.#pending.get(message.id);
    if (!pending) return;
    clearTimeout(pending.timeout);
    this.#pending.delete(message.id);
    if (message.error) pending.reject(new Error(`${message.error.message} (${message.error.code})`));
    else pending.resolve(message.result);
  }

  send(method: string, params: Record<string, unknown> = {}) {
    if (this.#closed) return Promise.reject(new Error("CDP 会话已关闭。"));
    return new Promise<unknown>((resolve, reject) => {
      const id = this.#nextId++;
      const timeout = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error(`CDP 命令超时：${method}`));
      }, 10000);
      this.#pending.set(id, { resolve, reject, timeout });
      this.#socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async evaluate(expression: string) {
    const result = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: false,
    }) as {
      result?: { value?: unknown };
      exceptionDetails?: { text?: string; exception?: { description?: string } };
    };
    if (result.exceptionDetails) {
      const detail = result.exceptionDetails.exception?.description ||
        result.exceptionDetails.text || "未知异常";
      throw new Error(`Multica 渲染页执行背景脚本失败：${detail.slice(0, 300)}`);
    }
    return result.result?.value;
  }

  close() {
    if (this.#closed) return;
    this.#closed = true;
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(new Error("CDP 会话已关闭。"));
    }
    this.#pending.clear();
    try { this.#socket.close(); } catch {}
  }
}

interface ManagedSession {
  session: CdpSession;
  earlyScriptId: string | null;
  revision: string | null;
}

export class InjectorEngine {
  #sessions = new Map<string, ManagedSession>();
  #timer: NodeJS.Timeout | null = null;
  #payload: string | null = null;
  #revision: string | null = null;
  #paused = false;
  #syncing = false;

  constructor(
    readonly port: number,
    readonly browserId: string,
    readonly onTargetCount: (count: number) => void,
  ) {}

  async start(payload: string, revision: string) {
    this.#payload = payload;
    this.#revision = revision;
    this.#paused = false;
    await this.#sync();
    if (!this.#timer) this.#timer = setInterval(() => void this.#sync(), 1200);
  }

  async update(payload: string, revision: string) {
    this.#payload = payload;
    this.#revision = revision;
    this.#paused = false;
    await this.#sync(true);
  }

  async pause() {
    this.#paused = true;
    await Promise.allSettled([...this.#sessions.values()].map((managed) => this.#remove(managed)));
  }

  async stop() {
    if (this.#timer) clearInterval(this.#timer);
    this.#timer = null;
    this.#paused = true;
    await Promise.allSettled([...this.#sessions.values()].map(async (managed) => {
      await this.#remove(managed);
      await managed.session.send("Page.setBypassCSP", { enabled: false }).catch(() => undefined);
      managed.session.close();
    }));
    this.#sessions.clear();
    this.onTargetCount(0);
  }

  async #remove(managed: ManagedSession) {
    if (managed.earlyScriptId) {
      await managed.session.send("Page.removeScriptToEvaluateOnNewDocument", {
        identifier: managed.earlyScriptId,
      }).catch(() => undefined);
      managed.earlyScriptId = null;
    }
    await managed.session.evaluate(REMOVE_RENDERER_PAYLOAD).catch(() => undefined);
    managed.revision = null;
  }

  async #apply(managed: ManagedSession) {
    if (!this.#payload || !this.#revision || this.#paused) return;
    if (managed.revision === this.#revision) return;
    if (managed.earlyScriptId) {
      await managed.session.send("Page.removeScriptToEvaluateOnNewDocument", {
        identifier: managed.earlyScriptId,
      }).catch(() => undefined);
    }
    await managed.session.send("Page.setBypassCSP", { enabled: true });
    const early = await managed.session.send("Page.addScriptToEvaluateOnNewDocument", {
      source: earlyPayloadFor(this.#payload, this.#revision),
    }) as { identifier?: string };
    managed.earlyScriptId = early.identifier ?? null;
    await managed.session.evaluate(this.#payload);
    managed.revision = this.#revision;
  }

  async #sync(force = false) {
    if (this.#syncing) return;
    this.#syncing = true;
    try {
      let targets: CdpTarget[];
      try {
        targets = await listTargets(this.port, this.browserId);
      } catch {
        // Multica 关闭或端口暂时不可达时保持现有会话，等下一轮重试
        this.onTargetCount(this.#sessions.size);
        return;
      }
      const targetIds = new Set(targets.map((target) => target.id));
      for (const [id, managed] of this.#sessions) {
        if (!targetIds.has(id) || managed.session.closed) {
          managed.session.close();
          this.#sessions.delete(id);
        }
      }
      for (const target of targets) {
        if (!this.#sessions.has(target.id)) {
          try {
            const session = await new CdpSession(target, this.port).open();
            const probe = await session.evaluate(`Boolean(
              document.querySelector("main.main-surface") ||
              document.querySelector("aside.app-shell-left-panel") ||
              document.documentElement
            )`);
            if (!probe) { session.close(); continue; }
            this.#sessions.set(target.id, { session, earlyScriptId: null, revision: null });
          } catch {}
        }
      }
      if (!this.#paused) {
        await Promise.allSettled([...this.#sessions.values()].map(async (managed) => {
          if (force) managed.revision = null;
          await this.#apply(managed);
        }));
      }
      this.onTargetCount(this.#sessions.size);
    } finally {
      this.#syncing = false;
    }
  }
}
