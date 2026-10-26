// Native "open file" dialog (tauri-plugin-dialog). In mock mode a stand-in
// picker returns a sample path, so screens work without Tauri; tests replace it.
import { isMock } from "./client";

export interface FileFilter {
  name: string;
  /** Without dots: `["png"]`. */
  extensions: string[];
}

export type FilePicker = (filters: FileFilter[]) => Promise<string | null>;

const MOCK_PATHS: Record<string, string> = {
  png: "C:\\Users\\me\\Pictures\\skin.png",
  exe: "C:\\Program Files\\Telegram Desktop\\Telegram.exe",
};

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
