/**
 * JSON serialization helper for SDK objects.
 *
 * Handles BigInt values (which JSON.stringify cannot serialize by default)
 * and produces pretty-printed JSON output suitable for CLI display.
 */

/**
 * Custom replacer for JSON.stringify that converts BigInt values to strings.
 */
function bigIntReplacer(_key: string, value: unknown): unknown {
  if (typeof value === 'bigint') {
    return value.toString()
  }
  return value
}

/**
 * Serialize a value to a pretty-printed JSON string.
 * Handles BigInt, undefined, and all standard JSON types.
 */
export function serialize(value: unknown): string {
  try {
    return JSON.stringify(value, bigIntReplacer, 2)
  } catch {
    return String(value)
  }
}

/**
 * Format a value for display in the CLI output.
 * Returns a pretty-printed JSON string representation.
 */
export function formatValue(value: unknown): string {
  if (value === undefined || value === null) {
    return 'null'
  }
  return serialize(value)
}
