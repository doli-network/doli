/**
 * Notifications store -- toast notifications for user feedback.
 * Uses Svelte 5 runes ($state).
 */

let nextId = 1;

export const notifications = $state([]);

/**
 * Add a notification toast.
 * @param {'success' | 'error' | 'info' | 'warning'} type
 * @param {string} message
 * @param {number} duration - Auto-dismiss after ms (0 = manual dismiss)
 */
export function addNotification(type, message, duration = 5000) {
  const id = nextId++;
  notifications.push({ id, type, message, timestamp: Date.now() });

  if (duration > 0) {
    setTimeout(() => {
      removeNotification(id);
    }, duration);
  }
}

/**
 * Remove a notification by id.
 * @param {number} id
 */
export function removeNotification(id) {
  const idx = notifications.findIndex((n) => n.id === id);
  if (idx !== -1) {
    notifications.splice(idx, 1);
  }
}

/**
 * Clear all notifications.
 */
export function clearNotifications() {
  notifications.length = 0;
}
