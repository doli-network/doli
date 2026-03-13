/**
 * Frontend input validation utilities.
 * These are UX-only validations -- the backend performs authoritative checks.
 */

const BECH32M_PREFIXES = ['doli', 'tdoli', 'ddoli'];
const HEX_PATTERN = /^[0-9a-fA-F]{64}$/;

/**
 * Validate a DOLI address (bech32m or hex pubkey hash).
 * @param {string} address
 * @returns {{ valid: boolean, error?: string }}
 */
export function validateAddress(address) {
  if (!address || !address.trim()) {
    return { valid: false, error: 'Address is required' };
  }
  const trimmed = address.trim();

  // Check bech32m format
  const isBech32m = BECH32M_PREFIXES.some((prefix) =>
    trimmed.startsWith(prefix + '1')
  );
  if (isBech32m) {
    // Basic length check (bech32m addresses are typically 40-70 chars)
    if (trimmed.length < 20 || trimmed.length > 100) {
      return { valid: false, error: 'Invalid bech32m address length' };
    }
    return { valid: true };
  }

  // Check hex format (64 char hex = 32 byte pubkey hash)
  if (HEX_PATTERN.test(trimmed)) {
    return { valid: true };
  }

  return { valid: false, error: 'Invalid address format. Use bech32m (doli1...) or hex pubkey hash.' };
}

/**
 * Validate a DOLI amount string.
 * @param {string} amount
 * @param {number} [maxUnits] - Maximum amount in base units (optional)
 * @returns {{ valid: boolean, error?: string }}
 */
export function validateAmount(amount) {
  if (!amount || !amount.trim()) {
    return { valid: false, error: 'Amount is required' };
  }
  const num = parseFloat(amount.trim());
  if (isNaN(num)) {
    return { valid: false, error: 'Invalid number' };
  }
  if (num <= 0) {
    return { valid: false, error: 'Amount must be greater than zero' };
  }
  // Check for too many decimal places (max 8 for DOLI)
  const parts = amount.trim().split('.');
  if (parts.length === 2 && parts[1].length > 8) {
    return { valid: false, error: 'Maximum 8 decimal places' };
  }
  return { valid: true };
}

/**
 * Validate a seed phrase (12 or 24 words).
 * @param {string} phrase
 * @returns {{ valid: boolean, error?: string }}
 */
export function validateSeedPhrase(phrase) {
  if (!phrase || !phrase.trim()) {
    return { valid: false, error: 'Seed phrase is required' };
  }
  const words = phrase.trim().split(/\s+/);
  if (words.length !== 12 && words.length !== 24) {
    return { valid: false, error: `Seed phrase must be 12 or 24 words (got ${words.length})` };
  }
  return { valid: true };
}

/**
 * Validate a wallet name.
 * @param {string} name
 * @returns {{ valid: boolean, error?: string }}
 */
export function validateWalletName(name) {
  if (!name || !name.trim()) {
    return { valid: false, error: 'Wallet name is required' };
  }
  if (name.trim().length > 64) {
    return { valid: false, error: 'Wallet name too long (max 64 characters)' };
  }
  return { valid: true };
}

/**
 * Validate a URL (for RPC endpoints).
 * @param {string} url
 * @returns {{ valid: boolean, error?: string }}
 */
export function validateUrl(url) {
  if (!url || !url.trim()) {
    return { valid: false, error: 'URL is required' };
  }
  try {
    const parsed = new URL(url.trim());
    if (!['http:', 'https:'].includes(parsed.protocol)) {
      return { valid: false, error: 'URL must use http:// or https://' };
    }
    return { valid: true };
  } catch {
    return { valid: false, error: 'Invalid URL format' };
  }
}

/**
 * Validate a hex string.
 * @param {string} hex
 * @param {number} [expectedBytes] - Expected byte length (hex chars / 2)
 * @returns {{ valid: boolean, error?: string }}
 */
export function validateHex(hex, expectedBytes) {
  if (!hex || !hex.trim()) {
    return { valid: false, error: 'Hex string is required' };
  }
  const trimmed = hex.trim();
  if (!/^[0-9a-fA-F]+$/.test(trimmed)) {
    return { valid: false, error: 'Invalid hex characters' };
  }
  if (trimmed.length % 2 !== 0) {
    return { valid: false, error: 'Hex string must have even length' };
  }
  if (expectedBytes !== undefined && trimmed.length !== expectedBytes * 2) {
    return { valid: false, error: `Expected ${expectedBytes} bytes (${expectedBytes * 2} hex chars)` };
  }
  return { valid: true };
}
