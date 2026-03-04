import Foundation

private let phraseFileName = "phrase"
private let historyFileName = "history.txt"

/// Handles mnemonic and history file storage.
struct CliPersistence {
    let dataDir: String

    /// Reads an existing mnemonic from the data directory,
    /// or generates a new 12-word BIP39 mnemonic and saves it.
    func getOrCreateMnemonic() throws -> String {
        let filename = (dataDir as NSString).appendingPathComponent(phraseFileName)

        if FileManager.default.fileExists(atPath: filename) {
            return try String(contentsOfFile: filename, encoding: .utf8)
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }

        let mnemonic = generateBip39Mnemonic()
        try mnemonic.write(toFile: filename, atomically: true, encoding: .utf8)
        return mnemonic
    }

    /// Returns the path to the REPL history file.
    func historyFile() -> String {
        (dataDir as NSString).appendingPathComponent(historyFileName)
    }

    /// Loads REPL history lines from disk.
    func loadHistory() -> [String] {
        let path = historyFile()
        guard FileManager.default.fileExists(atPath: path),
              let contents = try? String(contentsOfFile: path, encoding: .utf8) else {
            return []
        }
        return contents.components(separatedBy: "\n").filter { !$0.isEmpty }
    }

    /// Appends a single line to the history file.
    func appendHistory(_ line: String) {
        let path = historyFile()
        let entry = line + "\n"
        if let handle = FileHandle(forWritingAtPath: path) {
            handle.seekToEndOfFile()
            handle.write(entry.data(using: .utf8)!)
            handle.closeFile()
        } else {
            try? entry.write(toFile: path, atomically: true, encoding: .utf8)
        }
    }
}
