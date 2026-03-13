/**
 * Formatting utilities for the DOLI GUI.
 * Amounts use base units (u64) internally, formatted for display in the frontend.
 */

const UNITS_PER_DOLI = 100_000_000;

/**
 * Format base units as DOLI with 8 decimal places.
 * @param {number} units - Amount in base units
 * @returns {string} Formatted amount (e.g., "1.23456789 DOLI")
 */
export function formatBalance(units) {
  const coins = units / UNITS_PER_DOLI;
  return `${coins.toFixed(8)} DOLI`;
}

/**
 * Format base units as short DOLI (2-4 decimal places).
 * @param {number} units - Amount in base units
 * @returns {string} Short formatted amount
 */
export function formatBalanceShort(units) {
  const coins = units / UNITS_PER_DOLI;
  if (coins >= 1000) return `${coins.toFixed(2)} DOLI`;
  if (coins >= 1) return `${coins.toFixed(4)} DOLI`;
  return `${coins.toFixed(8)} DOLI`;
}

/**
 * Parse a DOLI amount string to base units.
 * @param {string} amount - Amount string (e.g., "1.5")
 * @returns {number} Amount in base units
 */
export function parseAmount(amount) {
  const coins = parseFloat(amount.trim());
  if (isNaN(coins) || coins < 0) throw new Error('Invalid amount');
  return Math.round(coins * UNITS_PER_DOLI);
}

/**
 * Truncate a hash or address for display.
 * @param {string} hash - Full hash string
 * @param {number} chars - Number of chars to show at start and end
 * @returns {string} Truncated string (e.g., "abc...xyz")
 */
export function truncateHash(hash, chars = 8) {
  if (!hash || hash.length <= chars * 2 + 3) return hash;
  return `${hash.slice(0, chars)}...${hash.slice(-chars)}`;
}

/**
 * Format a Unix timestamp as a localized date/time string.
 * @param {number} timestamp - Unix timestamp in seconds
 * @returns {string} Formatted date/time
 */
export function formatTimestamp(timestamp) {
  if (!timestamp) return 'Unknown';
  const date = new Date(timestamp * 1000);
  return date.toLocaleString();
}

/**
 * Format a number with comma separators.
 * @param {number} num - Number to format
 * @returns {string} Formatted number
 */
export function formatNumber(num) {
  if (num === null || num === undefined) return '0';
  return num.toLocaleString();
}
