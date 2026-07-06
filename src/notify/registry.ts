/** Maps self-assigned notification ids to the task they belong to, so a
 *  notification tap can be routed back to the right task. Ids are 32-bit
 *  positive integers (a plugin-notification constraint) and wrap on overflow. */
export interface NotificationRegistry {
  register(taskId: string): number;
  resolve(id: number): string | undefined;
}

export function createNotificationRegistry(): NotificationRegistry {
  let next = 1;
  const map = new Map<number, string>();
  return {
    register(taskId) {
      const id = next;
      next = next >= 0x7fffffff ? 1 : next + 1;
      map.set(id, taskId);
      return id;
    },
    resolve(id) {
      return map.get(id);
    },
  };
}
