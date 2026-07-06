/** Replace [selStart, selEnd) in `value` with `insert`; return new value and
 *  the cursor position just after the inserted text. */
export function insertAtCursor(
  value: string,
  selStart: number,
  selEnd: number,
  insert: string,
): { value: string; cursor: number } {
  const next = value.slice(0, selStart) + insert + value.slice(selEnd);
  return { value: next, cursor: selStart + insert.length };
}
