import 'dart:io';

/// Lightweight readline with tab completion and persistent history.
///
/// Uses raw stdin mode to handle key-by-key input, supporting:
/// - Tab completion (prefix matching)
/// - History navigation (up/down arrows, loaded/saved to file)
/// - Line editing (left/right, backspace, delete, Home/End, Ctrl-A/E/K/U/W)
class Readline {
  final List<String> _completions;
  final String? _historyFile;
  final List<String> _history = [];
  int _historyIndex = -1;
  String _savedLine = '';

  Readline({required List<String> completions, String? historyFile})
    : _completions = List.unmodifiable(completions),
      _historyFile = historyFile {
    _loadHistory();
  }

  /// Read a line of input with the given [prompt].
  ///
  /// Returns `null` on EOF (Ctrl-D on empty line).
  /// Throws [StdinException] on Ctrl-C.
  String? readLine(String prompt) {
    stdout.write(prompt);
    stdin.echoMode = false;
    stdin.lineMode = false;

    final buf = <int>[]; // character codes
    var cursor = 0;

    try {
      while (true) {
        final byte = stdin.readByteSync();
        if (byte == -1) return null;

        // Ctrl-D — EOF on empty line, delete-char otherwise
        if (byte == 4) {
          if (buf.isEmpty) return null;
          if (cursor < buf.length) {
            buf.removeAt(cursor);
            _redrawFrom(prompt, buf, cursor);
          }
          continue;
        }

        // Ctrl-C
        if (byte == 3) {
          stdout.writeln('^C');
          throw const StdinException('Interrupted');
        }

        // Enter
        if (byte == 10 || byte == 13) {
          stdout.writeln();
          final line = String.fromCharCodes(buf);
          if (line.trim().isNotEmpty) {
            _history.add(line);
          }
          _historyIndex = -1;
          return line;
        }

        // Tab — completion
        if (byte == 9) {
          _handleTab(prompt, buf, cursor, (b, c) {
            buf
              ..clear()
              ..addAll(b);
            cursor = c;
          });
          cursor = buf.length > cursor ? cursor : buf.length;
          continue;
        }

        // Backspace (127 or 8)
        if (byte == 127 || byte == 8) {
          if (cursor > 0) {
            cursor--;
            buf.removeAt(cursor);
            _redrawFrom(prompt, buf, cursor);
          }
          continue;
        }

        // Ctrl-A — home
        if (byte == 1) {
          _moveCursorTo(prompt, buf, cursor, 0);
          cursor = 0;
          continue;
        }

        // Ctrl-E — end
        if (byte == 5) {
          _moveCursorTo(prompt, buf, cursor, buf.length);
          cursor = buf.length;
          continue;
        }

        // Ctrl-K — kill to end of line
        if (byte == 11) {
          if (cursor < buf.length) {
            buf.removeRange(cursor, buf.length);
            _redrawFrom(prompt, buf, cursor);
          }
          continue;
        }

        // Ctrl-U — kill to start of line
        if (byte == 21) {
          if (cursor > 0) {
            buf.removeRange(0, cursor);
            cursor = 0;
            _redrawFrom(prompt, buf, cursor);
          }
          continue;
        }

        // Ctrl-W — kill word backward
        if (byte == 23) {
          if (cursor > 0) {
            var i = cursor - 1;
            while (i > 0 && buf[i - 1] == 32) {
              i--;
            }
            while (i > 0 && buf[i - 1] != 32) {
              i--;
            }
            buf.removeRange(i, cursor);
            cursor = i;
            _redrawFrom(prompt, buf, cursor);
          }
          continue;
        }

        // Escape sequence
        if (byte == 27) {
          final next = stdin.readByteSync();
          if (next == 91) {
            // CSI sequence
            final seq = stdin.readByteSync();
            switch (seq) {
              case 65: // Up arrow
                if (_history.isNotEmpty) {
                  if (_historyIndex == -1) {
                    _savedLine = String.fromCharCodes(buf);
                    _historyIndex = _history.length - 1;
                  } else if (_historyIndex > 0) {
                    _historyIndex--;
                  }
                  _replaceBuffer(prompt, buf, cursor, _history[_historyIndex].codeUnits);
                  cursor = buf.length;
                }
              case 66: // Down arrow
                if (_historyIndex >= 0) {
                  if (_historyIndex < _history.length - 1) {
                    _historyIndex++;
                    _replaceBuffer(prompt, buf, cursor, _history[_historyIndex].codeUnits);
                  } else {
                    _historyIndex = -1;
                    _replaceBuffer(prompt, buf, cursor, _savedLine.codeUnits);
                  }
                  cursor = buf.length;
                }
              case 67: // Right arrow
                if (cursor < buf.length) {
                  stdout.write('\x1b[C');
                  cursor++;
                }
              case 68: // Left arrow
                if (cursor > 0) {
                  stdout.write('\x1b[D');
                  cursor--;
                }
              case 72: // Home
                _moveCursorTo(prompt, buf, cursor, 0);
                cursor = 0;
              case 70: // End
                _moveCursorTo(prompt, buf, cursor, buf.length);
                cursor = buf.length;
              case 51: // Delete (ESC [ 3 ~)
                final tilde = stdin.readByteSync();
                if (tilde == 126 && cursor < buf.length) {
                  buf.removeAt(cursor);
                  _redrawFrom(prompt, buf, cursor);
                }
            }
          }
          continue;
        }

        // Regular printable character
        if (byte >= 32) {
          buf.insert(cursor, byte);
          cursor++;
          if (cursor == buf.length) {
            stdout.writeCharCode(byte);
          } else {
            _redrawFrom(prompt, buf, cursor);
          }
        }
      }
    } finally {
      stdin.echoMode = true;
      stdin.lineMode = true;
    }
  }

  /// Save history to file and release resources.
  void close() {
    _saveHistory();
  }

  // ---------------------------------------------------------------------------
  // Tab completion
  // ---------------------------------------------------------------------------

  void _handleTab(
    String prompt,
    List<int> buf,
    int cursor,
    void Function(List<int> newBuf, int newCursor) update,
  ) {
    final text = String.fromCharCodes(buf);
    final matches = _completions.where((c) => c.startsWith(text)).toList()..sort();

    if (matches.isEmpty) return;

    if (matches.length == 1) {
      // Single match — complete it with a trailing space.
      final completed = '${matches.first} ';
      update(completed.codeUnits.toList(), completed.length);
      _redrawLine(prompt, completed.codeUnits.toList(), completed.length);
    } else {
      // Multiple matches — find common prefix, complete to it,
      // then show all matches below.
      final common = _commonPrefix(matches);
      if (common.length > text.length) {
        update(common.codeUnits.toList(), common.length);
        _redrawLine(prompt, common.codeUnits.toList(), common.length);
      } else {
        // Show all matches below the prompt, then redraw.
        stdout.writeln();
        for (final m in matches) {
          stdout.write('  $m');
        }
        stdout.writeln();
        _redrawLine(prompt, buf, cursor);
      }
    }
  }

  String _commonPrefix(List<String> strings) {
    if (strings.isEmpty) return '';
    var prefix = strings.first;
    for (var i = 1; i < strings.length; i++) {
      while (!strings[i].startsWith(prefix)) {
        prefix = prefix.substring(0, prefix.length - 1);
        if (prefix.isEmpty) return '';
      }
    }
    return prefix;
  }

  // ---------------------------------------------------------------------------
  // History
  // ---------------------------------------------------------------------------

  void _loadHistory() {
    if (_historyFile == null) return;
    final file = File(_historyFile);
    if (!file.existsSync()) return;
    final lines = file.readAsLinesSync();
    _history.addAll(lines.where((l) => l.isNotEmpty));
  }

  void _saveHistory() {
    if (_historyFile == null) return;
    final file = File(_historyFile);
    // Keep last 500 entries.
    final entries = _history.length > 500 ? _history.sublist(_history.length - 500) : _history;
    file.writeAsStringSync('${entries.join('\n')}\n');
  }

  // ---------------------------------------------------------------------------
  // Terminal helpers
  // ---------------------------------------------------------------------------

  /// Redraw the entire line (prompt + buffer) and position cursor.
  void _redrawLine(String prompt, List<int> buf, int cursor) {
    stdout.write('\r\x1b[K$prompt${String.fromCharCodes(buf)}');
    // Move cursor back to correct position
    final back = buf.length - cursor;
    if (back > 0) {
      stdout.write('\x1b[${back}D');
    }
  }

  /// Clear from cursor to end, redraw remaining text, reposition cursor.
  void _redrawFrom(String prompt, List<int> buf, int cursor) {
    _redrawLine(prompt, buf, cursor);
  }

  /// Replace buffer contents and redraw.
  void _replaceBuffer(String prompt, List<int> buf, int oldCursor, List<int> newContent) {
    buf
      ..clear()
      ..addAll(newContent);
    _redrawLine(prompt, buf, newContent.length);
  }

  /// Move visible cursor from [from] to [to] position.
  void _moveCursorTo(String prompt, List<int> buf, int from, int to) {
    if (to < from) {
      stdout.write('\x1b[${from - to}D');
    } else if (to > from) {
      stdout.write('\x1b[${to - from}C');
    }
  }
}
