using OwO_Settings;

var root = Path.Combine(Path.GetTempPath(), "owo-dictionary-capacity-" + Guid.NewGuid().ToString("N"));
try {
    var store = new DictionaryStore(root, _ => Task.CompletedTask);
    var entries = Enumerable.Range(0, 100001).Select(i => new DictionaryWord(i.ToString("x32"), "kuorong", new string('测', 100) + i, 1)).ToArray();
    var dictionary = new ManagedDictionary(new string('a', 32), "扩容回归", "OwO JSON", "isolated", "full_pinyin", true, entries);
    var next = await store.SaveAsync(await store.LoadAsync(), [dictionary]);
    var loaded = await store.LoadAsync();
    if (loaded.Dictionaries.Single().Entries.Count != 100001) throw new Exception("Large catalog did not round-trip.");
    var snapshot = Path.Combine(root, "snapshots", File.ReadAllText(Path.Combine(root, "current")));
    if (new FileInfo(Path.Combine(snapshot, "catalog.json")).Length <= 32 * 1024 * 1024 || new FileInfo(Path.Combine(snapshot, "active.tsv")).Length <= 24 * 1024 * 1024)
        throw new Exception("Fixture did not cross both legacy byte limits.");
    var oversized = Enumerable.Range(0, DictionaryStore.MaximumEntries + 1).Select(i => new DictionaryWord(i.ToString("x32"), "ce", "测", 1)).ToArray();
    try { await store.SaveAsync(loaded, [dictionary with { Entries = oversized }]); throw new Exception("Entry cap was not enforced."); }
    catch (InvalidDataException) { }
    if ((await store.LoadAsync()).Revision != next.Revision) throw new Exception("Rejected input altered published snapshot.");
    Console.WriteLine("Capacity passed: 100001 entries; catalog >32 MiB and active >24 MiB round-trip; >500000 rejected without replacing existing data.");
} finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
