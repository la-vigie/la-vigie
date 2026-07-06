// Helpers for the per-file diff header. The diff parser (gitdiff-parser, via
// react-diff-view) leaves the absent side of a diff as "/dev/null": oldPath for
// an added file, newPath for a deleted one. Always display the real side.

export interface FilePathParts {
  type?: string;
  oldPath?: string;
  newPath?: string;
}

function isReal(p?: string): p is string {
  return !!p && p !== "/dev/null";
}

/** The path to show in a file header, picking the non-"/dev/null" side. */
export function displayPath(file: FilePathParts): string {
  if (isReal(file.newPath)) return file.newPath;
  if (isReal(file.oldPath)) return file.oldPath;
  return "unknown";
}

/** A short change label for the header, or null for a plain modification. */
export function fileChangeLabel(type?: string): string | null {
  switch (type) {
    case "add":
      return "added";
    case "delete":
      return "deleted";
    case "rename":
      return "renamed";
    case "copy":
      return "copied";
    default:
      return null;
  }
}
