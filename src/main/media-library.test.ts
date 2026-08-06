import { randomUUID } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

// 模拟随机图片 API：每次下载返回内容不同的合法 PNG
let downloadCounter = 0;
vi.mock("./network-policy.js", () => ({
  downloadRemoteMedia: vi.fn(async (url: string, temporaryDirectory: string) => {
    downloadCounter += 1;
    const bytes = Buffer.concat([makePngHeader(), Buffer.from([downloadCounter])]);
    const temporaryPath = path.join(temporaryDirectory, `${randomUUID()}.png.download`);
    await writeFile(temporaryPath, bytes);
    return {
      temporaryPath,
      originalName: `random-${downloadCounter}.png`,
      mimeType: "image/png",
      kind: "image" as const,
      byteSize: bytes.length,
      sourceUrl: url,
    };
  }),
}));

const { MediaLibrary } = await import("./media-library.js");

function makePngHeader() {
  const pngHeader = Buffer.alloc(24);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(pngHeader, 0);
  pngHeader.writeUInt32BE(13, 8);
  pngHeader.write("IHDR", 12, "ascii");
  pngHeader.writeUInt32BE(2, 16);
  pngHeader.writeUInt32BE(2, 20);
  return pngHeader;
}

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe("MediaLibrary", () => {
  it("copies managed media, deduplicates it, and removes it cleanly", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "multica-background-library-"));
    temporaryDirectories.push(root);
    const source = path.join(root, "中文背景.png");
    await writeFile(source, makePngHeader());
    const library = new MediaLibrary(path.join(root, "data"));
    await library.load();

    const first = await library.importFiles([source]);
    const duplicate = await library.importFiles([source]);
    expect(first.added).toHaveLength(1);
    expect(first.added[0].name).toBe("中文背景.png");
    expect(duplicate.added).toHaveLength(0);
    expect(duplicate.skipped[0].reason).toBe("媒体已存在");
    expect(await library.remove(first.added[0].id)).toBe(true);
    expect(library.items).toHaveLength(0);
  });

  it("stores random API sources and refreshes them in place", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "multica-background-api-"));
    temporaryDirectories.push(root);
    const library = new MediaLibrary(path.join(root, "data"));
    await library.load();

    const apiUrl = "https://example.com/api/random?sfw=true";
    const imported = await library.importRemote(apiUrl, { dynamic: true });
    expect(imported.added).toHaveLength(1);
    const item = imported.added[0];
    expect(item.origin).toBe("api");
    expect(item.sourceUrl).toBe(apiUrl);
    expect(item.name).toContain("example.com");

    const before = await readFile(library.pathFor(item));
    const refreshed = await library.refreshDynamic(item.id);
    expect(refreshed.id).toBe(item.id);
    expect(refreshed.sha256).not.toBe(item.sha256);
    const after = await readFile(library.pathFor(refreshed));
    expect(after.equals(before)).toBe(false);

    // 非 api 条目拒绝刷新
    const local = path.join(root, "local.png");
    await writeFile(local, Buffer.concat([makePngHeader(), Buffer.from([0xff])]));
    const localImport = await library.importFiles([local]);
    await expect(library.refreshDynamic(localImport.added[0].id)).rejects.toThrow("随机 API");
  });

  it("recursively discovers supported media without following symlinks", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "multica-background-folder-"));
    temporaryDirectories.push(root);
    await writeFile(path.join(root, "one.jpg"), "image");
    await writeFile(path.join(root, "ignore.txt"), "text");
    const library = new MediaLibrary(path.join(root, "data"));
    await library.load();
    expect(await library.discoverFolder(root)).toContain(path.join(root, "one.jpg"));
    expect(await library.discoverFolder(root)).not.toContain(path.join(root, "ignore.txt"));
  });
});
