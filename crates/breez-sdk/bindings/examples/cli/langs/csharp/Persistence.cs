using NBitcoin;

namespace BreezCli;

/// <summary>
/// Handles mnemonic and history file storage for the CLI.
/// </summary>
public class CliPersistence
{
    private const string PhraseFileName = "phrase";
    private const string HistoryFileName = "history.txt";

    public string DataDir { get; }

    public CliPersistence(string dataDir)
    {
        DataDir = dataDir;
    }

    /// <summary>
    /// Reads an existing mnemonic from the data directory, or generates a new
    /// 12-word BIP-39 mnemonic and saves it.
    /// </summary>
    public string GetOrCreateMnemonic()
    {
        var filename = Path.Combine(DataDir, PhraseFileName);

        if (File.Exists(filename))
        {
            return File.ReadAllText(filename).Trim();
        }

        var mnemonic = new Mnemonic(Wordlist.English, WordCount.Twelve);
        var phrase = mnemonic.ToString();

        Directory.CreateDirectory(DataDir);
        File.WriteAllText(filename, phrase);
        return phrase;
    }

    /// <summary>
    /// Returns the path to the REPL history file.
    /// </summary>
    public string HistoryFile()
    {
        return Path.Combine(DataDir, HistoryFileName);
    }

    /// <summary>
    /// Loads command history from file, returning an empty list if the file does not exist.
    /// </summary>
    public List<string> LoadHistory()
    {
        var historyPath = HistoryFile();
        if (File.Exists(historyPath))
        {
            return File.ReadAllLines(historyPath).ToList();
        }
        return new List<string>();
    }

    /// <summary>
    /// Saves command history to file.
    /// </summary>
    public void SaveHistory(List<string> history)
    {
        var historyPath = HistoryFile();
        Directory.CreateDirectory(Path.GetDirectoryName(historyPath)!);
        File.WriteAllLines(historyPath, history);
    }
}
