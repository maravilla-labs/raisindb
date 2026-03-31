/**
 * Configurable logger for RaisinDB client
 */

/**
 * Log levels in order of verbosity (lower = less verbose)
 */
export enum LogLevel {
  Silent = 0,
  Error = 1,
  Warn = 2,
  Info = 3,
  Debug = 4,
}

/**
 * Logger configuration
 */
export interface LoggerConfig {
  /** Current log level (default: Error) */
  level: LogLevel;
  /** Prefix for all log messages (default: "[RaisinDB]") */
  prefix: string;
}

/**
 * Global logger configuration
 */
let config: LoggerConfig = {
  level: LogLevel.Error,
  prefix: '[RaisinDB]',
};

/**
 * Configure the logger
 *
 * @param options - Logger configuration options
 *
 * @example
 * ```typescript
 * import { configureLogger, LogLevel } from '@raisindb/client';
 *
 * // Enable debug logging
 * configureLogger({ level: LogLevel.Debug });
 *
 * // Disable all logging
 * configureLogger({ level: LogLevel.Silent });
 * ```
 */
export function configureLogger(options: Partial<LoggerConfig>): void {
  config = { ...config, ...options };
}

/**
 * Get current logger configuration
 */
export function getLoggerConfig(): LoggerConfig {
  return { ...config };
}

/**
 * Set log level
 *
 * @param level - Log level to set
 */
export function setLogLevel(level: LogLevel): void {
  config.level = level;
}

/**
 * Get current log level
 */
export function getLogLevel(): LogLevel {
  return config.level;
}

/**
 * Logger instance with level-aware logging methods
 */
export const logger = {
  /**
   * Log debug message (only shown when level >= Debug)
   */
  debug(...args: unknown[]): void {
    if (config.level >= LogLevel.Debug) {
      console.log(config.prefix, ...args);
    }
  },

  /**
   * Log info message (only shown when level >= Info)
   */
  info(...args: unknown[]): void {
    if (config.level >= LogLevel.Info) {
      console.log(config.prefix, ...args);
    }
  },

  /**
   * Log warning message (only shown when level >= Warn)
   */
  warn(...args: unknown[]): void {
    if (config.level >= LogLevel.Warn) {
      console.warn(config.prefix, ...args);
    }
  },

  /**
   * Log error message (only shown when level >= Error)
   */
  error(...args: unknown[]): void {
    if (config.level >= LogLevel.Error) {
      console.error(config.prefix, ...args);
    }
  },
};

if (typeof globalThis !== 'undefined') {
  (globalThis as any).__raisinDebug = (enable = true) => {
    setLogLevel(enable ? LogLevel.Debug : LogLevel.Error);
    console.log(`[RaisinDB] Debug logging ${enable ? 'ENABLED' : 'DISABLED'} (includes server handler logs)`);
  };
}
