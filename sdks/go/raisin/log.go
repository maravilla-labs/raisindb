// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

package raisin

import "fmt"

// Log emits a structured log line at the given level. The line lands in the
// execution result and on the SSE log stream the CLI dev loop reads.
func Log(level LogLevel, format string, args ...any) {
	currentHost().Log(level, fmt.Sprintf(format, args...))
}

// Debug logs at debug level.
func Debug(format string, args ...any) { Log(LevelDebug, format, args...) }

// Info logs at info level.
func Info(format string, args ...any) { Log(LevelInfo, format, args...) }

// Warn logs at warn level.
func Warn(format string, args ...any) { Log(LevelWarn, format, args...) }

// Error logs at error level.
func Error(format string, args ...any) { Log(LevelError, format, args...) }
