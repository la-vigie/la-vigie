/** Trailing-edge debounce: coalesces rapid calls into one, `ms` after the last. */
export function debounce(fn: () => void, ms: number): (() => void) & { cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const wrapped = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = undefined;
      fn();
    }, ms);
  };
  wrapped.cancel = () => {
    if (timer) {
      clearTimeout(timer);
      timer = undefined;
    }
  };
  return wrapped;
}

/** Trailing-edge debounce with one independent timer per string key. */
export function keyedDebounce(fn: (key: string) => void, ms: number): (key: string) => void {
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  return (key: string) => {
    const existing = timers.get(key);
    if (existing) clearTimeout(existing);
    timers.set(
      key,
      setTimeout(() => {
        timers.delete(key);
        fn(key);
      }, ms),
    );
  };
}

/** Leading-edge throttle: fire immediately, then swallow calls for `ms`. */
export function throttleLeading(fn: () => void, ms: number): () => void {
  let cooling = false;
  return () => {
    if (cooling) return;
    fn();
    cooling = true;
    setTimeout(() => {
      cooling = false;
    }, ms);
  };
}
