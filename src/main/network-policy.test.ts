import { describe, expect, it } from "vitest";
import { isBlockedAddress, validateRemoteUrl } from "./network-policy.js";

describe("remote media network policy", () => {
  it.each([
    "127.0.0.1",
    "10.0.1.2",
    "172.20.1.2",
    "192.168.1.2",
    "169.254.169.254",
    "::1",
    "fc00::1",
    "fe80::1",
    "::ffff:127.0.0.1",
  ])("blocks non-public address %s", (address) => {
    expect(isBlockedAddress(address)).toBe(true);
  });

  it.each(["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"])("permits public address %s", (address) => {
    expect(isBlockedAddress(address)).toBe(false);
  });

  it("accepts a normal HTTPS URL", () => {
    expect(validateRemoteUrl("https://images.example.com/background.webp").href)
      .toBe("https://images.example.com/background.webp");
  });

  it.each([
    "file:///C:/secret.txt",
    "http://localhost:3000/image.png",
    "http://127.0.0.1/image.png",
    "https://user:pass@example.com/image.png",
  ])("rejects unsafe URL %s", (url) => {
    expect(() => validateRemoteUrl(url)).toThrow();
  });
});

