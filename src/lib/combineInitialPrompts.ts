/// Compose the launch prompt from a per-repo prefix and a per-task prompt.
/// Trims each, drops empties, joins repo-then-task with a blank line. Returns
/// undefined when nothing remains (launch a bare agent).
export function combineInitialPrompts(
  repoPrompt?: string | null,
  taskPrompt?: string | null,
): string | undefined {
  const parts = [repoPrompt, taskPrompt]
    .map((p) => (p ?? "").trim())
    .filter((p) => p.length > 0);
  return parts.length > 0 ? parts.join("\n\n") : undefined;
}
