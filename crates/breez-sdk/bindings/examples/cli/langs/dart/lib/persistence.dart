import 'dart:io';

import 'package:bip39/bip39.dart' as bip39;

const _phraseFileName = 'phrase';
const _historyFileName = 'history.txt';

class CliPersistence {
  final String dataDir;

  CliPersistence(this.dataDir);

  String getOrCreateMnemonic() {
    final phraseFile = File('$dataDir/$_phraseFileName');
    if (phraseFile.existsSync()) {
      return phraseFile.readAsStringSync().trim();
    }
    final words = bip39.generateMnemonic(strength: 128); // 12 words
    phraseFile.writeAsStringSync(words);
    return words;
  }

  String get historyFile => '$dataDir/$_historyFileName';
}
