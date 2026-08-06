import { createHash, randomUUID } from "node:crypto";
import { createReadStream, promises as fs } from "node:fs";
import path from "node:path";
import { imageSize } from "image-size";
import { ImportResult, MediaItem, MediaKind } from "../shared/contracts.js";
import { downloadRemoteMedia } from "./network-policy.js";

const MEDIA_TYPES: Record<string, { kind: MediaKind; mimeType: string; maximum: number }> = {
  ".png": { kind: "image", mimeType: "image/png", maximum: 50 * 1024 * 1024 },
  ".jpg": { kind: "image", mimeType: "image/jpeg", maximum: 50 * 1024 * 1024 },
  ".jpeg": { kind: "image", mimeType: "image/jpeg", maximum: 50 * 1024 * 1024 },
  ".webp": { kind: "image", mimeType: "image/webp", maximum: 50 * 1024 * 1024 },
  ".gif": { kind: "image", mimeType: "image/gif", maximum: 50 * 1024 * 1024 },
  ".avif": { kind: "image", mimeType: "image/avif", maximum: 50 * 1024 * 1024 },
  ".mp4": { kind: "video", mimeType: "video/mp4", maximum: 1024 * 1024 * 1024 },
  ".webm": { kind: "video", mimeType: "video/webm", maximum: 1024 * 1024 * 1024 },
  ".ogv": { kind: "video", mimeType: "video/ogg", maximum: 1024 * 1024 * 1024 },
  ".mov": { kind: "video", mimeType: "video/quicktime", maximum: 1024 * 1024 * 1024 },
};

const safeDisplayName = (name: string) =>
  name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_").trim().slice(0, 180) || "未命名媒体";

async function sha256(filePath: string) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(filePath)) hash.update(chunk as Buffer);
  return hash.digest("hex");
}

async function validateMediaFile(filePath: string, mediaType: { kind: MediaKind }, extension: string) {
  if (mediaType.kind === "image") {
    const bytes = await fs.readFile(filePath);
    let dimensions;
    try { dimensions = imageSize(bytes); } catch { throw new Error("图片内容损坏或格式与扩展名不匹配。"); }
    const width = Number(dimensions.width);
    const height = Number(dimensions.height);
    if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1 ||
      width > 16_384 || height > 16_384 || width * height > 50_000_000) {
      throw new Error("图片尺寸超过 16384 像素或 5000 万总像素上限。");
    }
    const expectedTypes: Record<string, string[]> = {
      ".jpg": ["jpg", "jpeg"], ".jpeg": ["jpg", "jpeg"], ".png": ["png"],
      ".webp": ["webp"], ".gif": ["gif"], ".avif": ["avif", "heif"],
    };
    if (dimensions.type && !expectedTypes[extension]?.includes(dimensions.type.toLowerCase())) {
      throw new Error("图片内容与文件扩展名不匹配。");
    }
    return;
  }
  const handle = await fs.open(filePath, "r");
  try {
    const header = Buffer.alloc(64);
    const { bytesRead } = await handle.read(header, 0, header.length, 0);
    const bytes = header.subarray(0, bytesRead);
    const valid = extension === ".webm"
      ? bytes.length >= 4 && bytes.subarray(0, 4).equals(Buffer.from([0x1a, 0x45, 0xdf, 0xa3]))
      : extension === ".ogv"
        ? bytes.length >= 4 && bytes.subarray(0, 4).toString("ascii") === "OggS"
        : bytes.length >= 12 && bytes.subarray(4, 8).toString("ascii") === "ftyp";
    if (!valid) throw new Error("视频内容损坏或格式与扩展名不匹配。");
  } finally {
    await handle.close();
  }
}

async function writeCatalog(filePath: string, items: MediaItem[]) {
  const temporary = `${filePath}.${process.pid}.${Date.now()}.tmp`;
  await fs.writeFile(temporary, `${JSON.stringify(items, null, 2)}\n`, { encoding: "utf8", flag: "wx" });
  await fs.rm(filePath, { force: true });
  await fs.rename(temporary, filePath);
}

export class MediaLibrary {
  readonly mediaDirectory: string;
  readonly temporaryDirectory: string;
  readonly catalogPath: string;
  #items: MediaItem[] = [];

  constructor(readonly dataDirectory: string) {
    this.mediaDirectory = path.join(dataDirectory, "media");
    this.temporaryDirectory = path.join(dataDirectory, "temporary");
    this.catalogPath = path.join(dataDirectory, "library.json");
  }

  get items() {
    return structuredClone(this.#items);
  }

  getById(id: string) {
    const item = this.#items.find((candidate) => candidate.id === id);
    return item ? structuredClone(item) : null;
  }

  pathFor(item: MediaItem) {
    const full = path.resolve(this.mediaDirectory, item.fileName);
    const relative = path.relative(this.mediaDirectory, full);
    if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) {
      throw new Error("媒体目录校验失败。");
    }
    return full;
  }

  async load() {
    await Promise.all([
      fs.mkdir(this.mediaDirectory, { recursive: true }),
      fs.mkdir(this.temporaryDirectory, { recursive: true }),
    ]);
    try {
      const raw = JSON.parse(await fs.readFile(this.catalogPath, "utf8"));
      if (!Array.isArray(raw)) throw new Error("Invalid catalog");
      const valid: MediaItem[] = [];
      for (const entry of raw) {
        if (!entry || typeof entry !== "object" || typeof entry.id !== "string" ||
          typeof entry.fileName !== "string" || typeof entry.sha256 !== "string") continue;
        const item = entry as MediaItem;
        try {
          const stat = await fs.stat(this.pathFor(item));
          if (stat.isFile()) valid.push(item);
        } catch {}
      }
      this.#items = valid;
      if (valid.length !== raw.length) await writeCatalog(this.catalogPath, valid);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
        await fs.rename(this.catalogPath, `${this.catalogPath}.invalid-${Date.now()}`).catch(() => undefined);
      }
      this.#items = [];
      await writeCatalog(this.catalogPath, []);
    }
    return this.items;
  }

  async #ingest(
    sourcePath: string,
    details: {
      name?: string;
      origin: "local" | "remote" | "api";
      sourceUrl?: string;
      removeSource?: boolean;
      allowDuplicate?: boolean;
      /** 显式扩展名；显示名不含扩展名（如随机 API 条目）时必须提供 */
      extension?: string;
    },
  ) {
    const source = path.resolve(sourcePath);
    const extension = (details.extension ?? path.extname(details.name || source)).toLowerCase();
    const mediaType = MEDIA_TYPES[extension];
    if (!mediaType) throw new Error("不支持此图片或视频格式。");
    const stat = await fs.stat(source);
    if (!stat.isFile() || stat.size < 1) throw new Error("媒体文件为空或不可读取。");
    if (stat.size > mediaType.maximum) {
      throw new Error(`媒体文件超过 ${Math.round(mediaType.maximum / 1024 / 1024)} MB 上限。`);
    }
    await validateMediaFile(source, mediaType, extension);
    const digest = await sha256(source);
    const duplicate = details.allowDuplicate ? null : this.#items.find((item) => item.sha256 === digest);
    if (duplicate) {
      if (details.removeSource) await fs.rm(source, { force: true });
      return { item: duplicate, duplicate: true };
    }
    const id = randomUUID();
    const storedName = `${id}${extension}`;
    const target = path.join(this.mediaDirectory, storedName);
    const temporary = path.join(this.mediaDirectory, `.${id}.incoming`);
    try {
      if (details.removeSource) await fs.rename(source, temporary).catch(async () => {
        await fs.copyFile(source, temporary);
        await fs.rm(source, { force: true });
      });
      else await fs.copyFile(source, temporary);
      const copied = await fs.stat(temporary);
      if (copied.size !== stat.size) throw new Error("媒体复制校验失败。");
      await fs.rename(temporary, target);
      const item: MediaItem = {
        id,
        name: safeDisplayName(details.name || path.basename(source)),
        kind: mediaType.kind,
        origin: details.origin,
        fileName: storedName,
        mimeType: mediaType.mimeType,
        byteSize: stat.size,
        sha256: digest,
        sourceUrl: details.sourceUrl,
        createdAt: new Date().toISOString(),
      };
      this.#items.unshift(item);
      await writeCatalog(this.catalogPath, this.#items);
      return { item, duplicate: false };
    } catch (error) {
      await fs.rm(temporary, { force: true });
      await fs.rm(target, { force: true });
      throw error;
    }
  }

  async importFiles(filePaths: string[]): Promise<ImportResult> {
    const result: ImportResult = { added: [], skipped: [] };
    for (const filePath of filePaths) {
      try {
        const imported = await this.#ingest(filePath, { origin: "local" });
        if (imported.duplicate) result.skipped.push({ path: filePath, reason: "媒体已存在" });
        else result.added.push(imported.item);
      } catch (error) {
        result.skipped.push({ path: filePath, reason: (error as Error).message });
      }
    }
    return result;
  }

  async discoverFolder(folderPath: string) {
    const root = path.resolve(folderPath);
    const pending = [root];
    const files: string[] = [];
    while (pending.length > 0) {
      const directory = pending.shift()!;
      for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
        if (entry.isSymbolicLink()) continue;
        const full = path.join(directory, entry.name);
        if (entry.isDirectory()) pending.push(full);
        else if (entry.isFile() && MEDIA_TYPES[path.extname(entry.name).toLowerCase()]) files.push(full);
        if (files.length + pending.length > 10_000) throw new Error("文件夹内容过多，请选择更具体的目录。");
      }
    }
    return files;
  }

  async importRemote(url: string, options: { dynamic?: boolean } = {}): Promise<ImportResult> {
    let downloaded: Awaited<ReturnType<typeof downloadRemoteMedia>> | null = null;
    try {
      downloaded = await downloadRemoteMedia(url, this.temporaryDirectory);
      const dynamic = Boolean(options.dynamic);
      const imported = await this.#ingest(downloaded.temporaryPath, {
        // 动态源保存用户输入的 API 地址（而非重定向后的最终地址），刷新时重新请求它；
        // 且允许与库中已有图片同哈希，因为条目本身才是「源」
        name: dynamic ? `随机 API · ${new URL(url).hostname}` : downloaded.originalName,
        origin: dynamic ? "api" : "remote",
        sourceUrl: dynamic ? url : downloaded.sourceUrl,
        removeSource: true,
        allowDuplicate: dynamic,
        extension: path.extname(downloaded.originalName),
      });
      return imported.duplicate
        ? { added: [], skipped: [{ path: url, reason: "媒体已存在" }] }
        : { added: [imported.item], skipped: [] };
    } catch (error) {
      if (downloaded) await fs.rm(downloaded.temporaryPath, { force: true });
      return { added: [], skipped: [{ path: url, reason: (error as Error).message }] };
    }
  }

  /** 重新请求随机 API，替换该条目的媒体内容（条目 id 保持不变） */
  async refreshDynamic(id: string): Promise<MediaItem> {
    const index = this.#items.findIndex((candidate) => candidate.id === id);
    if (index < 0) throw new Error("媒体项目不存在。");
    const item = this.#items[index];
    if (item.origin !== "api" || !item.sourceUrl) throw new Error("该媒体不是随机 API 来源。");

    const downloaded = await downloadRemoteMedia(item.sourceUrl, this.temporaryDirectory);
    try {
      const extension = path.extname(downloaded.originalName).toLowerCase();
      const mediaType = MEDIA_TYPES[extension];
      if (!mediaType) throw new Error("不支持此图片或视频格式。");
      await validateMediaFile(downloaded.temporaryPath, mediaType, extension);
      const digest = await sha256(downloaded.temporaryPath);
      if (digest === item.sha256) {
        await fs.rm(downloaded.temporaryPath, { force: true });
        return structuredClone(item);
      }

      const previousPath = this.pathFor(item);
      // 用内容哈希做文件名而非覆盖同名旧文件：旧文件可能正被媒体服务器读取，
      // Windows 下覆盖/删除被占用文件会 EBUSY。写新文件后旧文件仅作最佳努力清理。
      const storedName = `${item.id}-${digest.slice(0, 12)}${extension}`;
      const target = path.join(this.mediaDirectory, storedName);
      await fs.rename(downloaded.temporaryPath, target).catch(async () => {
        await fs.copyFile(downloaded.temporaryPath, target);
        await fs.rm(downloaded.temporaryPath, { force: true });
      });
      if (path.resolve(previousPath) !== path.resolve(target)) {
        await fs.rm(previousPath, { force: true }).catch(() => undefined);
      }

      const updated: MediaItem = {
        ...item,
        fileName: storedName,
        mimeType: downloaded.mimeType,
        kind: downloaded.kind,
        byteSize: downloaded.byteSize,
        sha256: digest,
      };
      this.#items[index] = updated;
      await writeCatalog(this.catalogPath, this.#items);
      return structuredClone(updated);
    } catch (error) {
      await fs.rm(downloaded.temporaryPath, { force: true });
      throw error;
    }
  }

  async remove(id: string) {
    const item = this.#items.find((candidate) => candidate.id === id);
    if (!item) return false;
    this.#items = this.#items.filter((candidate) => candidate.id !== id);
    await writeCatalog(this.catalogPath, this.#items);
    await fs.rm(this.pathFor(item), { force: true });
    return true;
  }
}
