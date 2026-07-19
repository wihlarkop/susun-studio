import { describe, expect, it } from "vitest";
import { formatBytes } from "./utils";

describe("formatBytes", () => {
  it("shows sub-kilobyte sizes as whole bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
  });

  it("steps up through KB/MB/GB/TB as the value grows", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatBytes(2 * 1024 * 1024 * 1024)).toBe("2.0 GB");
  });

  it("caps at TB instead of inventing a larger unit", () => {
    expect(formatBytes(3 * 1024 * 1024 * 1024 * 1024)).toBe("3.0 TB");
    expect(formatBytes(4096 * 1024 * 1024 * 1024 * 1024)).toBe("4096.0 TB");
  });
});
