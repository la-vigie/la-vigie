import { open } from "@tauri-apps/plugin-dialog";
import { importCustomSound, deleteCustomSound } from "../api";
import { getSoundUrl, decodeTest } from "./source";
import { ALLOWED_SOUND_EXTS, type CustomSound } from "./types";

/**
 * Pick a local audio file and register it as a custom sound. The backend
 * validates extension + size and copies the file; we then decode-test the
 * managed copy in an Audio element (the truest "will it play?" check) and roll
 * the import back if it fails. Returns null if the user cancels the picker.
 */
export async function pickAndImportSound(): Promise<CustomSound | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Audio", extensions: [...ALLOWED_SOUND_EXTS] }],
  });
  if (typeof selected !== "string") return null; // cancelled

  const stem = selected.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? "sound";
  const entry = await importCustomSound(selected, stem);

  // Validate by actually decoding the copied file.
  const url = await getSoundUrl(entry.id, [entry]);
  const ok = url ? await decodeTest(url) : false;
  if (!ok) {
    try {
      await deleteCustomSound(entry.id);
    } catch {
      // Ignore rollback failures — the import failed regardless.
    }
    throw new Error("That file could not be played and was not added.");
  }
  return entry;
}
