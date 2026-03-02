from pathlib import Path

from mnemonic import Mnemonic

PHRASE_FILE_NAME = "phrase"
HISTORY_FILE_NAME = "history.txt"


class CliPersistence:
    def __init__(self, data_dir: Path):
        self.data_dir = Path(data_dir)

    def get_or_create_mnemonic(self) -> str:
        phrase_file = self.data_dir / PHRASE_FILE_NAME
        if phrase_file.exists():
            return phrase_file.read_text().strip()
        m = Mnemonic("english")
        words = m.generate(strength=128)  # 12 words
        phrase_file.write_text(words)
        return words

    def history_file(self) -> str:
        return str(self.data_dir / HISTORY_FILE_NAME)
