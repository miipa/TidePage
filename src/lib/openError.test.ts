import { describe, it, expect } from "vitest";
import { COVER_FAIL_KEY, classifyOpenError, openErrorKey } from "./openError";

describe("classifyOpenError", () => {
  it("missing", () => {
    expect(classifyOpenError("Error: 书籍不存在")).toBe("missing");
    expect(classifyOpenError("No such file or directory (os error 2)")).toBe("missing");
  });

  it("permission", () => {
    expect(classifyOpenError("Access is denied. (os error 5)")).toBe("permission");
    expect(classifyOpenError("拒绝访问")).toBe("permission");
  });

  it("damaged", () => {
    expect(classifyOpenError("InvalidPDFException: Invalid PDF structure.")).toBe("damaged");
    expect(classifyOpenError("ParseTimeoutOrCorrupted: EPUB 解析超时或文件损坏")).toBe("damaged");
  });

  it("unsupported", () => {
    expect(classifyOpenError("unsupported format")).toBe("unsupported");
    expect(classifyOpenError("PalmDoc 加密")).toBe("unsupported");
  });

  it("unknown 兜底且永不返回原始串", () => {
    const err = new Error("Some internal stack trace xyz");
    const kind = classifyOpenError(err);
    expect(kind).toBe("unknown");
    const key = openErrorKey(err);
    expect(key).toBe("reader.err.openUnknown");
    expect(key).not.toContain("stack trace");
    expect(COVER_FAIL_KEY).toBe("toast.library.coverFailed");
  });

  it("空异常 → unknown", () => {
    expect(classifyOpenError(null)).toBe("unknown");
    expect(classifyOpenError(undefined)).toBe("unknown");
  });
});
