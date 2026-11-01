using System;
using System.Collections.Generic;
using System.IO;
using System.Text;

// Reproducible ZIP32/stored writer for PowerShell 5.1 and 7. Framework's
// CompressionLevel.NoCompression can still emit method 8; the portable host
// needs method 0, without adding a Deflate dependency to the input method.
public static class OwOBundleStoredZip
{
    private sealed class Entry {
        public byte[] Name;
        public uint Crc, Size, Offset;
    }
    private static readonly uint[] Table = CrcTable();
    private static uint[] CrcTable() {
        var table = new uint[256];
        for (uint i = 0; i < 256; ++i) {
            uint value = i;
            for (int bit = 0; bit < 8; ++bit) value = (value & 1) != 0 ? (value >> 1) ^ 0xedb88320U : value >> 1;
            table[i] = value;
        }
        return table;
    }
    public static void Create(string source, string destination) {
        string root = Path.GetFullPath(source).TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
        var files = Directory.GetFiles(root, "*", SearchOption.AllDirectories);
        Array.Sort(files, StringComparer.Ordinal);
        if (files.Length == 0 || files.Length > 4096) throw new InvalidDataException("Invalid plugin file count.");
        var entries = new List<Entry>();
        var buffer = new byte[65536];
        using (var output = new FileStream(destination, FileMode.CreateNew, FileAccess.Write, FileShare.None))
        using (var writer = new BinaryWriter(output, Encoding.UTF8)) {
            foreach (string path in files) {
                string relative = path.Substring(root.Length + 1).Replace('\\', '/');
                var name = Encoding.UTF8.GetBytes(relative);
                if (name.Length > ushort.MaxValue) throw new InvalidDataException("Plugin ZIP name is too long.");
                using (var input = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read)) {
                    if (input.Length > 256L * 1024 * 1024) throw new InvalidDataException("Plugin file is too large.");
                    uint crc = uint.MaxValue;
                    int count;
                    while ((count = input.Read(buffer, 0, buffer.Length)) > 0)
                        for (int i = 0; i < count; ++i) crc = Table[(crc ^ buffer[i]) & 255] ^ (crc >> 8);
                    var entry = new Entry { Name = name, Crc = ~crc, Size = checked((uint)input.Length), Offset = checked((uint)output.Position) };
                    input.Position = 0;
                    writer.Write(0x04034b50U); writer.Write((ushort)20); writer.Write((ushort)0x800);
                    writer.Write((ushort)0); writer.Write((ushort)0); writer.Write((ushort)0x21);
                    writer.Write(entry.Crc); writer.Write(entry.Size); writer.Write(entry.Size);
                    writer.Write((ushort)name.Length); writer.Write((ushort)0); writer.Write(name);
                    writer.Flush(); input.CopyTo(output); entries.Add(entry);
                    if (output.Position > 256L * 1024 * 1024) throw new InvalidDataException("Plugin package is too large.");
                }
            }
            uint centralOffset = checked((uint)output.Position);
            foreach (var entry in entries) {
                writer.Write(0x02014b50U); writer.Write((ushort)20); writer.Write((ushort)20);
                writer.Write((ushort)0x800); writer.Write((ushort)0); writer.Write((ushort)0); writer.Write((ushort)0x21);
                writer.Write(entry.Crc); writer.Write(entry.Size); writer.Write(entry.Size);
                writer.Write((ushort)entry.Name.Length); writer.Write((ushort)0); writer.Write((ushort)0);
                writer.Write((ushort)0); writer.Write((ushort)0); writer.Write(0U); writer.Write(entry.Offset); writer.Write(entry.Name);
            }
            uint centralSize = checked((uint)output.Position - centralOffset);
            writer.Write(0x06054b50U); writer.Write((ushort)0); writer.Write((ushort)0);
            writer.Write((ushort)entries.Count); writer.Write((ushort)entries.Count);
            writer.Write(centralSize); writer.Write(centralOffset); writer.Write((ushort)0);
        }
    }
}
