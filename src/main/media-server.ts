import { randomBytes } from "node:crypto";
import { createReadStream, promises as fs } from "node:fs";
import http, { IncomingMessage, ServerResponse } from "node:http";
import { MediaLibrary } from "./media-library.js";

function send(response: ServerResponse, status: number, body = "") {
  response.writeHead(status, {
    "Content-Type": "text/plain; charset=utf-8",
    "Cache-Control": "no-store",
    "Cross-Origin-Resource-Policy": "cross-origin",
    "Access-Control-Allow-Origin": "*",
  });
  response.end(body);
}

export class MediaServer {
  readonly token = randomBytes(32).toString("hex");
  #server: http.Server | null = null;
  #port = 0;

  constructor(readonly library: MediaLibrary) {}

  get origin() {
    if (!this.#port) throw new Error("媒体服务尚未启动。");
    return `http://127.0.0.1:${this.#port}`;
  }

  urlFor(id: string) {
    return `${this.origin}/${this.token}/media/${encodeURIComponent(id)}`;
  }

  async start() {
    if (this.#server) return this.origin;
    this.#server = http.createServer((request, response) => void this.#handle(request, response));
    this.#server.on("clientError", (_error, socket) => socket.end("HTTP/1.1 400 Bad Request\r\n\r\n"));
    await new Promise<void>((resolve, reject) => {
      this.#server!.once("error", reject);
      this.#server!.listen(0, "127.0.0.1", () => {
        this.#server!.off("error", reject);
        const address = this.#server!.address();
        if (!address || typeof address === "string") return reject(new Error("媒体服务端口分配失败。"));
        this.#port = address.port;
        resolve();
      });
    });
    return this.origin;
  }

  async stop() {
    const server = this.#server;
    this.#server = null;
    this.#port = 0;
    if (!server) return;
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }

  async #handle(request: IncomingMessage, response: ServerResponse) {
    if (request.method !== "GET" && request.method !== "HEAD") return send(response, 405);
    let url: URL;
    try {
      url = new URL(request.url ?? "/", this.origin);
    } catch {
      return send(response, 400);
    }
    const match = url.pathname.match(/^\/([0-9a-f]{64})\/media\/([0-9a-f-]{36})$/i);
    if (!match || match[1] !== this.token) return send(response, 404);
    const item = this.library.getById(match[2]);
    if (!item) return send(response, 404);
    const filePath = this.library.pathFor(item);
    let stat;
    try {
      stat = await fs.stat(filePath);
    } catch {
      return send(response, 404);
    }
    const headers: Record<string, string | number> = {
      "Content-Type": item.mimeType,
      "Accept-Ranges": "bytes",
      "Cache-Control": "private, max-age=3600",
      "Cross-Origin-Resource-Policy": "cross-origin",
      "Access-Control-Allow-Origin": "*",
      "X-Content-Type-Options": "nosniff",
    };
    const range = request.headers.range?.match(/^bytes=(\d*)-(\d*)$/);
    if (range) {
      const requestedStart = range[1] ? Number(range[1]) : 0;
      const requestedEnd = range[2] ? Number(range[2]) : stat.size - 1;
      if (!Number.isInteger(requestedStart) || !Number.isInteger(requestedEnd) ||
        requestedStart < 0 || requestedEnd < requestedStart || requestedStart >= stat.size) {
        response.writeHead(416, { "Content-Range": `bytes */${stat.size}` });
        response.end();
        return;
      }
      const end = Math.min(requestedEnd, stat.size - 1);
      headers["Content-Range"] = `bytes ${requestedStart}-${end}/${stat.size}`;
      headers["Content-Length"] = end - requestedStart + 1;
      response.writeHead(206, headers);
      if (request.method === "HEAD") return response.end();
      createReadStream(filePath, { start: requestedStart, end }).pipe(response);
      return;
    }
    headers["Content-Length"] = stat.size;
    response.writeHead(200, headers);
    if (request.method === "HEAD") return response.end();
    createReadStream(filePath).pipe(response);
  }
}
