/**
 * Utility functions for the Social Feed SSR app
 */

/**
 * Format a date string to a relative time (e.g., "2 hours ago")
 */
export function formatRelativeTime(dateString: string): string {
  const date = new Date(dateString);
  const now = new Date();
  const diffInSeconds = Math.floor((now.getTime() - date.getTime()) / 1000);

  if (diffInSeconds < 60) {
    return 'just now';
  }

  const diffInMinutes = Math.floor(diffInSeconds / 60);
  if (diffInMinutes < 60) {
    return `${diffInMinutes}m ago`;
  }

  const diffInHours = Math.floor(diffInMinutes / 60);
  if (diffInHours < 24) {
    return `${diffInHours}h ago`;
  }

  const diffInDays = Math.floor(diffInHours / 24);
  if (diffInDays < 7) {
    return `${diffInDays}d ago`;
  }

  const diffInWeeks = Math.floor(diffInDays / 7);
  if (diffInWeeks < 4) {
    return `${diffInWeeks}w ago`;
  }

  // Format as date
  return date.toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    year: date.getFullYear() !== now.getFullYear() ? 'numeric' : undefined,
  });
}

/**
 * Truncate text to a maximum length
 */
export function truncate(text: string, maxLength: number): string {
  if (text.length <= maxLength) {
    return text;
  }
  return text.slice(0, maxLength) + '...';
}

/**
 * Class names helper (simple version of clsx)
 */
export function cn(...classes: (string | undefined | null | false)[]): string {
  return classes.filter(Boolean).join(' ');
}

/**
 * Generate avatar URL or initials
 */
export function getAvatarUrl(username: string, avatar?: string): string {
  if (avatar) {
    return avatar;
  }
  // Generate a placeholder avatar with initials
  const initials = username.slice(0, 2).toUpperCase();
  return `https://ui-avatars.com/api/?name=${initials}&background=6366f1&color=fff`;
}

/**
 * Format number with abbreviations (e.g., 1500 -> 1.5k)
 */
export function formatNumber(num: number): string {
  if (num < 1000) {
    return num.toString();
  }
  if (num < 1000000) {
    return (num / 1000).toFixed(1).replace(/\.0$/, '') + 'k';
  }
  return (num / 1000000).toFixed(1).replace(/\.0$/, '') + 'M';
}
