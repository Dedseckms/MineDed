// Native "save as" dialog (tauri-plugin-dialog). In mock mode a stand-in accepts
// the suggested name, so the screen works without Tauri; tests replace it.
import { isMock } from "./client";

export interface SaveOptions {
  /** Suggested file name or path; its folder is pre-selected in the dialog. */
  defaultPath?: string;
  /** Without dots: `["png", "jpg"]`. */
  extensions?: string[];
}

export type SavePicker = (options: SaveOptions) => Promise<string | null>;

export interface FileFilter {
  name: string;
  /** Without dots: `["png"]`. */
  extensions: string[];
}

export type FilePicker = (filters: FileFilter[]) => Promise<string | null>;

/** Asks for several files; `[]` when the user cancels. */
export type FilesPicker = (filters: FileFilter[]) => Promise<string[]>;

const MOCK_PATHS: Record<string, string> = {
  png: "C:\\Users\\me\\Pictures\\skin.png",
  exe: "C:\\Program Files\\Telegram Desktop\\Telegram.exe",
  mrpack: "C:\\Users\\me\\Downloads\\Fabulously Optimized 6.4.0.mrpack",
};

const MOCK_MULTI: Record<string, string[]> = {
  jar: ["C:\\Users\\me\\Downloads\\my-mod-1.0.jar", "C:\\Users\\me\\Downloads\\another-mod-2.3.jar"],
  zip: ["C:\\Users\\me\\Downloads\\My Pack 32x.zip"],
};

const defaultMockFilesPicker: FilesPicker = async (filters) => {
  const ext = filters[0]?.extensions[0] ?? "";
  return MOCK_MULTI[ext] ?? [];
};

let mockFilesPicker: FilesPicker = defaultMockFilesPicker;

/** Mock mode only: replaces the stand-in multi-file picker (`null` restores the default). */
export function setMockFilesPicker(picker: FilesPicker | null): void {
  mockFilesPicker = picker ?? defaultMockFilesPicker;
}

/** Asks for one or more files; `[]` when the user cancels. */
export async function pickFiles(filters: FileFilter[] = []): Promise<string[]> {
  if (isMock) return mockFilesPicker(filters);
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({ multiple: true, directory: false, filters: filters.length > 0 ? filters : undefined });
  if (res === null) return [];
  return Array.isArray(res) ? res : [res];
}

const defaultMockPicker: FilePicker = async (filters) => {
  const ext = filters[0]?.extensions[0] ?? "";
  return MOCK_PATHS[ext] ?? "C:\\Users\\me\\Downloads\\subscription.yaml";
};

let mockPicker: FilePicker = defaultMockPicker;

/** Mock mode only: replaces the stand-in picker (`null` restores the default). */
export function setMockFilePicker(picker: FilePicker | null): void {
  mockPicker = picker ?? defaultMockPicker;
}

/** Asks for one file; `null` when the user cancels. Empty `filters` = any file. */
export async function pickFile(filters: FileFilter[] = []): Promise<string | null> {
  if (isMock) return mockPicker(filters);
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({ multiple: false, directory: false, filters: filters.length > 0 ? filters : undefined });
  return typeof res === "string" ? res : null;
}

function folderOf(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut > 0 ? path.slice(0, cut + 1) : "";
}

const defaultMockSave: SavePicker = async (options) => {
  const suggested = options?.defaultPath || "document.txt";
  return `${folderOf(suggested) || "C:/Downloads/"}${suggested.replace(/^.*[\\/]/, "")}`;
};

let mockSave: SavePicker = defaultMockSave;

/** Mock mode only: replaces the stand-in save dialog (`null` restores the default). */
export function setMockSavePicker(picker: SavePicker | null): void {
  mockSave = picker ?? defaultMockSave;
}

/**
 * Asks where to save and returns the chosen path (`null` when cancelled). The
 * native dialog opens with `defaultPath` filled in, so the file keeps the name
 * and extension offered here.
 */
export async function pickSavePath(options: SaveOptions = {}): Promise<string | null> {
  if (isMock) return mockSave(options);
  const { save } = await import("@tauri-apps/plugin-dialog");
  const res = await save({
    defaultPath: options.defaultPath,
    filters: options.extensions?.length
      ? [{ name: "File", extensions: options.extensions }]
      : undefined,
  });
  return typeof res === "string" ? res : null;
}
