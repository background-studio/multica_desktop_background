import dns from "node:dns";
import fs from "node:fs";
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { randomUUID } from "node:crypto";

const MAX_IMAGE_BYTES = 50 * 1024 * 1024;
const MAX_VIDEO_BYTES = 1024 * 1024 * 1024;
const MAX_REDIRECTS = 5;

const CONTENT_TYPE_EXTENSIONS: Record<string, { extension: string; kind: "image" | "video" }> = {
  "image/png": { extension: ".png", kind: "image" },
  "image/jpeg": { extension: ".jpg", kind: "image" },
  "image/webp": { extension: ".webp", kind: "image" },
  "image/gif": { extension: ".gif", kind: "image" },
  "image/avif": { extension: ".avif", kind: "image" },
  "video/mp4": { extension: ".mp4", kind: "video" },
  "video/webm": { extension: ".webm", kind: "video" },
  "video/ogg": { extension: ".ogv", kind: "video" },
  "video/quicktime": { extension: ".mov", kind: "video" },
};

function parseIpv4(address: string) {
  const parts = address.split(".").map(Number);
  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)) {
    return null;
  }
  return ((parts[0] * 0x1000000) + (parts[1] << 16) + (parts[2] << 8) + parts[3]) >>> 0;
}

const inIpv4Range = (value: number, base: number, prefix: number) => {
  const mask = prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0;
  return (value & mask) === (base & mask);
};

function expandIpv6(address: string) {
  let normalized = address.toLowerCase().split("%")[0];
  const mapped = normalized.match(/^(.*:)(\d+\.\d+\.\d+\.\d+)$/);
  if (mapped) {
    const ipv4 = parseIpv4(mapped[2]);
    if (ipv4 === null) return null;
    normalized = `${mapped[1]}${((ipv4 >>> 16) & 0xffff).toString(16)}:${(ipv4 & 0xffff).toString(16)}`;
  }
  const halves = normalized.split("::");
  if (halves.length > 2) return null;
  const left = halves[0] ? halves[0].split(":") : [];
  const right = halves[1] ? halves[1].split(":") : [];
  const missing = 8 - left.length - right.length;
  if (missing < 0 || (halves.length === 1 && missing !== 0)) return null;
  const groups = [...left, ...Array(missing).fill("0"), ...right];
  if (groups.length !== 8 || groups.some((group) => !/^[0-9a-f]{1,4}$/.test(group))) return null;
  return groups.map((group) => Number.parseInt(group, 16));
}

export function isBlockedAddress(address: string) {
  const family = net.isIP(address.split("%")[0]);
  if (family === 4) {
    const value = parseIpv4(address);
    if (value === null) return true;
    return [
      ["0.0.0.0", 8], ["10.0.0.0", 8], ["100.64.0.0", 10], ["127.0.0.0", 8],
      ["169.254.0.0", 16], ["172.16.0.0", 12], ["192.0.0.0", 24], ["192.0.2.0", 24],
      ["192.168.0.0", 16], ["198.18.0.0", 15], ["198.51.100.0", 24], ["203.0.113.0", 24],
      ["224.0.0.0", 4], ["240.0.0.0", 4],
    ].some(([base, prefix]) => inIpv4Range(value, parseIpv4(base as string)!, prefix as number));
  }
  if (family === 6) {
    const groups = expandIpv6(address);
    if (!groups) return true;
    const [first, second, third, fourth, fifth, sixth] = groups;
    if (groups.every((group) => group === 0)) return true;
    if (groups.slice(0, 7).every((group) => group === 0) && groups[7] === 1) return true;
    if ((first & 0xfe00) === 0xfc00 || (first & 0xffc0) === 0xfe80 || (first & 0xff00) === 0xff00) return true;
    if (first === 0x2001 && second === 0x0db8) return true;
    if (first === 0 && second === 0 && third === 0 && fourth === 0 && fifth === 0 && sixth === 0xffff) {
      const ipv4 = `${groups[6] >>> 8}.${groups[6] & 255}.${groups[7] >>> 8}.${groups[7] & 255}`;
      return isBlockedAddress(ipv4);
    }
    return false;
  }
  return true;
}

export function validateRemoteUrl(value: string) {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error("请输入有效的网络地址。");
  }
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) {
    throw new Error("仅支持不含账号信息的 HTTP 或 HTTPS 地址。");
  }
  if (!url.hostname || url.hostname.toLowerCase() === "localhost") {
    throw new Error("不允许访问本机或私有网络地址。");
  }
  if (net.isIP(url.hostname) && isBlockedAddress(url.hostname)) {
    throw new Error("不允许访问本机、私有或保留网络地址。");
  }
  return url;
}

async function checkedLookup(hostname: string) {
  const addresses = await dns.promises.lookup(hostname, { all: true, verbatim: true });
  const publicAddresses = addresses.filter((entry) => !isBlockedAddress(entry.address));
  if (publicAddresses.length !== addresses.length || publicAddresses.length === 0) {
    throw new Error("目标域名解析到了本机、私有或保留网络地址，已拒绝下载。");
  }
  return publicAddresses;
}

function fileNameFromHeaders(url: URL, headers: Record<string, string | string[] | undefined>, extension: string) {
  const disposition = Array.isArray(headers["content-disposition"])
    ? headers["content-disposition"][0]
    : headers["content-disposition"];
  const encodedName = disposition?.match(/filename\*=UTF-8''([^;]+)/i)?.[1];
  const simpleName = disposition?.match(/filename="?([^";]+)"?/i)?.[1];
  const candidate = encodedName ? decodeURIComponent(encodedName) : simpleName || path.basename(url.pathname);
  const safe = candidate.replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_").trim();
  if (!safe || safe === ".") return `remote-media${extension}`;
  const stem = path.basename(safe, path.extname(safe)).slice(0, Math.max(1, 180 - extension.length));
  return `${stem || "remote-media"}${extension}`;
}

export interface RemoteDownload {
  temporaryPath: string;
  originalName: string;
  mimeType: string;
  kind: "image" | "video";
  byteSize: number;
  sourceUrl: string;
}

export async function downloadRemoteMedia(value: string, temporaryDirectory = os.tmpdir()): Promise<RemoteDownload> {
  const initial = validateRemoteUrl(value);
  await fs.promises.mkdir(temporaryDirectory, { recursive: true });

  const download = async (url: URL, redirects: number): Promise<RemoteDownload> => {
    if (redirects > MAX_REDIRECTS) throw new Error("网络媒体重定向次数过多。");
    const addresses = await checkedLookup(url.hostname);
    const request = url.protocol === "https:" ? httpsRequest : httpRequest;
    return new Promise<RemoteDownload>((resolve, reject) => {
      const selected = addresses[0];
      // Node 20+ 的 autoSelectFamily 会以 all:true 调用 lookup 并期望地址数组，
      // 必须按调用方要求的形状返回，否则报 "Invalid IP address: undefined"
      const lookup = ((_hostname: string, options: dns.LookupOptions, callback: (...args: never[]) => void) => {
        if (options?.all) {
          (callback as unknown as (err: null, addresses: dns.LookupAddress[]) => void)(
            null,
            addresses.map((entry) => ({ address: entry.address, family: entry.family })),
          );
        } else {
          (callback as unknown as (err: null, address: string, family: number) => void)(
            null, selected.address, selected.family,
          );
        }
      }) as unknown as net.LookupFunction;
      const req = request(url, {
        method: "GET",
        headers: {
          "User-Agent": "Multica-Background-Studio/0.1",
          "Accept": "image/*,video/*;q=0.9",
        },
        lookup,
      }, async (response) => {
        const status = response.statusCode ?? 0;
        if (status >= 300 && status < 400 && response.headers.location) {
          response.resume();
          try {
            const redirected = validateRemoteUrl(new URL(response.headers.location, url).href);
            resolve(await download(redirected, redirects + 1));
          } catch (error) {
            reject(error);
          }
          return;
        }
        if (status < 200 || status >= 300) {
          response.resume();
          reject(new Error(`下载失败，服务器返回 HTTP ${status}。`));
          return;
        }
        const mimeType = String(response.headers["content-type"] ?? "").split(";")[0].trim().toLowerCase();
        const mediaType = CONTENT_TYPE_EXTENSIONS[mimeType];
        if (!mediaType) {
          response.resume();
          reject(new Error(`不支持服务器返回的媒体类型：${mimeType || "未知"}。`));
          return;
        }
        const limit = mediaType.kind === "image" ? MAX_IMAGE_BYTES : MAX_VIDEO_BYTES;
        const declaredSize = Number(response.headers["content-length"] ?? 0);
        if (Number.isFinite(declaredSize) && declaredSize > limit) {
          response.resume();
          reject(new Error(`媒体文件超过 ${Math.round(limit / 1024 / 1024)} MB 上限。`));
          return;
        }
        const temporaryPath = path.join(temporaryDirectory, `${randomUUID()}${mediaType.extension}.download`);
        const output = fs.createWriteStream(temporaryPath, { flags: "wx" });
        let byteSize = 0;
        let settled = false;
        const fail = async (error: Error) => {
          if (settled) return;
          settled = true;
          req.destroy();
          response.destroy();
          output.destroy();
          await fs.promises.rm(temporaryPath, { force: true }).catch(() => undefined);
          reject(error);
        };
        response.on("data", (chunk: Buffer) => {
          byteSize += chunk.length;
          if (byteSize > limit) void fail(new Error(`媒体文件超过 ${Math.round(limit / 1024 / 1024)} MB 上限。`));
        });
        response.on("error", (error) => void fail(error));
        output.on("error", (error) => void fail(error));
        output.on("finish", () => {
          if (settled) return;
          settled = true;
          output.close(() => resolve({
            temporaryPath,
            originalName: fileNameFromHeaders(url, response.headers, mediaType.extension),
            mimeType,
            kind: mediaType.kind,
            byteSize,
            sourceUrl: url.href,
          }));
        });
        response.pipe(output);
      });
      req.setTimeout(30_000, () => req.destroy(new Error("下载连接超时。")));
      req.on("error", reject);
      req.end();
    });
  };

  return download(initial, 0);
}
