using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using SharpCompress.Archives;
using SharpCompress.Common;
using SharpCompress.Readers;

namespace ArchiveHelper.Services
{
    /// <summary>
    /// Stream wrapper that masks CanSeek = false so ZipArchive streams sequentially
    /// using standard ZIP data descriptors rather than seeking backwards to rewrite local headers
    /// on every entry, which prevents OS write cache flushes and dramatically accelerates SSD throughput.
    /// </summary>
    internal sealed class SequentialWriteStream : Stream
    {
        private readonly Stream _inner;
        public SequentialWriteStream(Stream inner) => _inner = inner;
        public override bool CanRead => false;
        public override bool CanSeek => false;
        public override bool CanWrite => _inner.CanWrite;
        public override long Length => _inner.Length;
        public override long Position { get => _inner.Position; set => throw new NotSupportedException(); }
        public override void Flush() => _inner.Flush();
        public override Task FlushAsync(CancellationToken cancellationToken) => _inner.FlushAsync(cancellationToken);
        public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
        public override void Write(byte[] buffer, int offset, int count) => _inner.Write(buffer, offset, count);
        public override void Write(ReadOnlySpan<byte> buffer) => _inner.Write(buffer);
        public override Task WriteAsync(byte[] buffer, int offset, int count, CancellationToken cancellationToken) => _inner.WriteAsync(buffer, offset, count, cancellationToken);
        public override ValueTask WriteAsync(ReadOnlyMemory<byte> buffer, CancellationToken cancellationToken = default) => _inner.WriteAsync(buffer, cancellationToken);
        protected override void Dispose(bool disposing)
        {
            if (disposing) _inner.Dispose();
            base.Dispose(disposing);
        }
    }

    public readonly struct FolderProgressReport
    {
        public string ItemName { get; init; }
        public long BytesProcessed { get; init; }
        public long TotalBytes { get; init; }
        public int FilesProcessed { get; init; }
        public int TotalFiles { get; init; }
        public string? CurrentFileName { get; init; }
    }

    public class ArchiveService
    {
        public Task<(bool success, string message)> ZipItemAsync(
            string sourcePath,
            bool deleteOriginal,
            bool predictExt = false,
            string? destinationDir = null,
            bool keepMtime = true,
            CancellationToken ct = default,
            IProgress<FolderProgressReport>? progress = null)
        {
            return Task.Run(() =>
            {
                try
                {
                    if (Directory.Exists(sourcePath))
                    {
                        var dirInfo = new DirectoryInfo(sourcePath);
                        var allFiles = dirInfo.EnumerateFiles("*", SearchOption.AllDirectories).ToList();
                        if (allFiles.Count == 0 && !dirInfo.EnumerateFileSystemInfos().Any())
                        {
                            return (false, $"Skipped empty: {dirInfo.Name}");
                        }

                        if (!string.IsNullOrWhiteSpace(destinationDir))
                        {
                            string fullDest = Path.GetFullPath(destinationDir);
                            string fullSrc = Path.GetFullPath(dirInfo.FullName);
                            if (fullDest.Equals(fullSrc, StringComparison.OrdinalIgnoreCase) || 
                                fullDest.StartsWith(fullSrc + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase))
                            {
                                return (false, $"Cannot output zip inside the folder being zipped: {dirInfo.Name}");
                            }
                            Directory.CreateDirectory(destinationDir);
                        }

                        string extension = ".zip";
                        if (predictExt)
                        {
                            bool hasOsu = allFiles.Any(f => f.Extension.Equals(".osu", StringComparison.OrdinalIgnoreCase));
                            if (hasOsu)
                            {
                                extension = ".osz";
                            }
                            else
                            {
                                bool hasIni = allFiles.Any(f => f.Extension.Equals(".ini", StringComparison.OrdinalIgnoreCase) &&
                                                                !f.Name.Equals("desktop.ini", StringComparison.OrdinalIgnoreCase));
                                if (hasIni)
                                {
                                    extension = ".osk";
                                }
                            }
                        }

                        string zipName = $"{dirInfo.Name}{extension}";
                        string zipPath = !string.IsNullOrWhiteSpace(destinationDir)
                            ? Path.Combine(destinationDir, zipName)
                            : $"{dirInfo.FullName}{extension}";

                        if (File.Exists(zipPath))
                        {
                            SafeDeleteFile(zipPath);
                        }

                        var originalTime = dirInfo.LastWriteTime;
                        long totalBytes = allFiles.Sum(f => f.Length);
                        int totalFiles = allFiles.Count;

                        progress?.Report(new FolderProgressReport
                        {
                            ItemName = dirInfo.Name,
                            BytesProcessed = 0,
                            TotalBytes = totalBytes,
                            FilesProcessed = 0,
                            TotalFiles = totalFiles
                        });

                        long bytesWritten = 0;
                        int filesWritten = 0;
                        DateTime lastReport = DateTime.UtcNow;

                        const int StreamBufferSize = 1024 * 1024;
                        using (var zipFs = new FileStream(zipPath, FileMode.Create, FileAccess.Write, FileShare.None, StreamBufferSize, FileOptions.SequentialScan))
                        using (var seqStream = new SequentialWriteStream(zipFs))
                        using (var archive = new ZipArchive(seqStream, ZipArchiveMode.Create, leaveOpen: false))
                        {
                            byte[] buffer = new byte[StreamBufferSize];
                            foreach (var file in allFiles)
                            {
                                ct.ThrowIfCancellationRequested();
                                string relativePath = Path.GetRelativePath(dirInfo.FullName, file.FullName);
                                var entry = archive.CreateEntry(relativePath, CompressionLevel.NoCompression);
                                if (keepMtime)
                                {
                                    entry.LastWriteTime = file.LastWriteTime;
                                }

                                using (var entryStream = entry.Open())
                                using (var fileStream = new FileStream(file.FullName, FileMode.Open, FileAccess.Read, FileShare.Read, StreamBufferSize, FileOptions.SequentialScan))
                                {
                                    int read;
                                    while ((read = fileStream.Read(buffer, 0, buffer.Length)) > 0)
                                    {
                                        entryStream.Write(buffer, 0, read);
                                        bytesWritten += read;

                                        var now = DateTime.UtcNow;
                                        if ((now - lastReport).TotalMilliseconds >= 40)
                                        {
                                            lastReport = now;
                                            progress?.Report(new FolderProgressReport
                                            {
                                                ItemName = dirInfo.Name,
                                                BytesProcessed = bytesWritten,
                                                TotalBytes = totalBytes,
                                                FilesProcessed = filesWritten,
                                                TotalFiles = totalFiles,
                                                CurrentFileName = file.Name
                                            });
                                        }
                                    }
                                }
                                filesWritten++;
                            }
                        }

                        progress?.Report(new FolderProgressReport
                        {
                            ItemName = dirInfo.Name,
                            BytesProcessed = totalBytes,
                            TotalBytes = totalBytes,
                            FilesProcessed = totalFiles,
                            TotalFiles = totalFiles
                        });

                        if (keepMtime)
                        {
                            File.SetLastWriteTime(zipPath, originalTime);
                        }

                        if (deleteOriginal)
                        {
                            try
                            {
                                SafeDeleteDirectory(dirInfo.FullName);
                            }
                            catch (Exception delEx)
                            {
                                return (true, $"Zipped: {dirInfo.Name}{extension} (Warning: Could not delete original folder: {delEx.Message})");
                            }
                        }

                        return (true, $"Zipped: {dirInfo.Name}{extension}");
                    }
                    else if (File.Exists(sourcePath))
                    {
                        var fileInfo = new FileInfo(sourcePath);
                        if (!string.IsNullOrWhiteSpace(destinationDir))
                        {
                            Directory.CreateDirectory(destinationDir);
                        }

                        string zipName = $"{fileInfo.Name}.zip";
                        string zipPath = !string.IsNullOrWhiteSpace(destinationDir)
                            ? Path.Combine(destinationDir, zipName)
                            : $"{fileInfo.FullName}.zip";

                        if (File.Exists(zipPath))
                        {
                            SafeDeleteFile(zipPath);
                        }

                        var originalTime = fileInfo.LastWriteTime;
                        long totalBytes = fileInfo.Length;

                        progress?.Report(new FolderProgressReport
                        {
                            ItemName = fileInfo.Name,
                            BytesProcessed = 0,
                            TotalBytes = totalBytes,
                            FilesProcessed = 0,
                            TotalFiles = 1
                        });

                        const int SingleStreamBufferSize = 1024 * 1024;
                        using (var zipFs = new FileStream(zipPath, FileMode.Create, FileAccess.Write, FileShare.None, SingleStreamBufferSize, FileOptions.SequentialScan))
                        using (var seqStream = new SequentialWriteStream(zipFs))
                        using (var archive = new ZipArchive(seqStream, ZipArchiveMode.Create, leaveOpen: false))
                        {
                            var entry = archive.CreateEntry(fileInfo.Name, CompressionLevel.NoCompression);
                            if (keepMtime)
                            {
                                entry.LastWriteTime = originalTime;
                            }

                            using var entryStream = entry.Open();
                            using var fileStream = new FileStream(fileInfo.FullName, FileMode.Open, FileAccess.Read, FileShare.Read, SingleStreamBufferSize, FileOptions.SequentialScan);
                            byte[] buffer = new byte[SingleStreamBufferSize];
                            long bytesWritten = 0;
                            DateTime lastReport = DateTime.UtcNow;
                            int read;

                            while ((read = fileStream.Read(buffer, 0, buffer.Length)) > 0)
                            {
                                entryStream.Write(buffer, 0, read);
                                bytesWritten += read;

                                var now = DateTime.UtcNow;
                                if ((now - lastReport).TotalMilliseconds >= 40)
                                {
                                    lastReport = now;
                                    progress?.Report(new FolderProgressReport
                                    {
                                        ItemName = fileInfo.Name,
                                        BytesProcessed = bytesWritten,
                                        TotalBytes = totalBytes,
                                        FilesProcessed = 0,
                                        TotalFiles = 1,
                                        CurrentFileName = fileInfo.Name
                                    });
                                }
                            }
                        }

                        progress?.Report(new FolderProgressReport
                        {
                            ItemName = fileInfo.Name,
                            BytesProcessed = totalBytes,
                            TotalBytes = totalBytes,
                            FilesProcessed = 1,
                            TotalFiles = 1
                        });

                        if (keepMtime)
                        {
                            File.SetLastWriteTime(zipPath, originalTime);
                        }

                        if (deleteOriginal)
                        {
                            try
                            {
                                SafeDeleteFile(fileInfo.FullName);
                            }
                            catch (Exception delEx)
                            {
                                return (true, $"Zipped: {fileInfo.Name}.zip (Warning: Could not delete original file: {delEx.Message})");
                            }
                        }

                        return (true, $"Zipped: {fileInfo.Name}.zip");
                    }
                    else
                    {
                        return (false, $"Item not found: {sourcePath}");
                    }
                }
                catch (Exception ex)
                {
                    return (false, $"Error zipping {Path.GetFileName(sourcePath)}: {ex.Message}");
                }
            }, ct);
        }

        public Task<(bool success, string message)> UnzipArchiveAsync(
            string archivePath,
            bool deleteOriginal,
            bool keepTopDir = true,
            string? destinationDir = null,
            bool keepMtime = true,
            CancellationToken ct = default,
            IProgress<FolderProgressReport>? progress = null)
        {
            return Task.Run(() =>
            {
                try
                {
                    if (!File.Exists(archivePath))
                    {
                        return (false, $"Archive not found: {archivePath}");
                    }

                    var archiveFile = new FileInfo(archivePath);
                    var baseDir = !string.IsNullOrWhiteSpace(destinationDir)
                        ? destinationDir
                        : (archiveFile.DirectoryName ?? Path.GetPathRoot(archivePath) ?? "");
                    var folderName = Path.GetFileNameWithoutExtension(archivePath);
                    var destDir = keepTopDir ? Path.Combine(baseDir, folderName) : baseDir;

                    Directory.CreateDirectory(destDir);
                    var originalTime = archiveFile.LastWriteTime;

                    var ext = archiveFile.Extension.ToLowerInvariant();
                    bool extracted = false;

                    // If it's a standard .zip, .osk, or .osz file, use streamed parallel ZipArchive with progress
                    if (ext == ".zip" || ext == ".osk" || ext == ".osz")
                    {
                        try
                        {
                            var entryItems = new List<(string FullName, long Length, DateTimeOffset? LastWriteTime)>();
                            long totalBytes = 0;
                            using (var zipStream = new FileStream(archivePath, FileMode.Open, FileAccess.Read, FileShare.ReadWrite, 256 * 1024, FileOptions.None))
                            using (var archive = new ZipArchive(zipStream, ZipArchiveMode.Read))
                            {
                                foreach (var entry in archive.Entries)
                                {
                                    if (!string.IsNullOrEmpty(entry.Name))
                                    {
                                        entryItems.Add((entry.FullName, entry.Length, keepMtime ? entry.LastWriteTime : null));
                                        totalBytes += entry.Length;
                                    }
                                    else
                                    {
                                        string dirPath = Path.Combine(destDir, entry.FullName);
                                        Directory.CreateDirectory(dirPath);
                                    }
                                }
                            }

                            // Pre-create all distinct directories in destDir upfront so threads don't lock
                            var distinctDirs = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
                            foreach (var item in entryItems)
                            {
                                string target = Path.Combine(destDir, item.FullName);
                                string? dir = Path.GetDirectoryName(target);
                                if (!string.IsNullOrEmpty(dir))
                                {
                                    distinctDirs.Add(dir);
                                }
                            }
                            foreach (var dir in distinctDirs)
                            {
                                Directory.CreateDirectory(dir);
                            }

                            int totalFiles = entryItems.Count;
                            progress?.Report(new FolderProgressReport
                            {
                                ItemName = archiveFile.Name,
                                BytesProcessed = 0,
                                TotalBytes = totalBytes,
                                FilesProcessed = 0,
                                TotalFiles = totalFiles
                            });

                            long bytesExtracted = 0;
                            int filesExtracted = 0;
                            long lastReportTicks = DateTime.UtcNow.Ticks;
                            const int ExtractBufferSize = 512 * 1024; // 512 KB buffer for NVMe queue saturation

                            int degree = totalFiles >= 4
                                ? Math.Min(Environment.ProcessorCount, 8)
                                : 1;

                            var partitioner = Partitioner.Create(entryItems);
                            Parallel.ForEach(partitioner, new ParallelOptions { MaxDegreeOfParallelism = degree, CancellationToken = ct }, () =>
                            {
                                var stream = new FileStream(archivePath, FileMode.Open, FileAccess.Read, FileShare.ReadWrite, ExtractBufferSize, FileOptions.SequentialScan);
                                var localArchive = new ZipArchive(stream, ZipArchiveMode.Read);
                                byte[] localBuf = new byte[ExtractBufferSize];
                                return (stream, localArchive, localBuf);
                            },
                            (item, loopState, state) =>
                            {
                                ct.ThrowIfCancellationRequested();
                                var entry = state.localArchive.GetEntry(item.FullName);
                                if (entry != null)
                                {
                                    string entryTargetPath = Path.Combine(destDir, entry.FullName);
                                    using (var inStream = entry.Open())
                                    using (var outFile = new FileStream(entryTargetPath, FileMode.Create, FileAccess.Write, FileShare.None, ExtractBufferSize, FileOptions.SequentialScan))
                                    {
                                        int read;
                                        while ((read = inStream.Read(state.localBuf, 0, state.localBuf.Length)) > 0)
                                        {
                                            outFile.Write(state.localBuf, 0, read);
                                            long curBytes = Interlocked.Add(ref bytesExtracted, read);

                                            long nowTicks = DateTime.UtcNow.Ticks;
                                            long lastTicks = Interlocked.Read(ref lastReportTicks);
                                            if (nowTicks - lastTicks >= TimeSpan.TicksPerMillisecond * 40)
                                            {
                                                if (Interlocked.CompareExchange(ref lastReportTicks, nowTicks, lastTicks) == lastTicks)
                                                {
                                                    int curFiles = Volatile.Read(ref filesExtracted);
                                                    progress?.Report(new FolderProgressReport
                                                    {
                                                        ItemName = archiveFile.Name,
                                                        BytesProcessed = curBytes,
                                                        TotalBytes = totalBytes,
                                                        FilesProcessed = curFiles,
                                                        TotalFiles = totalFiles,
                                                        CurrentFileName = entry.Name
                                                    });
                                                }
                                            }
                                        }
                                    }

                                    if (keepMtime && item.LastWriteTime.HasValue)
                                    {
                                        try { File.SetLastWriteTime(entryTargetPath, item.LastWriteTime.Value.DateTime); } catch { }
                                    }
                                }

                                Interlocked.Increment(ref filesExtracted);
                                return state;
                            },
                            state =>
                            {
                                state.localArchive.Dispose();
                                state.stream.Dispose();
                            });

                            progress?.Report(new FolderProgressReport
                            {
                                ItemName = archiveFile.Name,
                                BytesProcessed = totalBytes,
                                TotalBytes = totalBytes,
                                FilesProcessed = totalFiles,
                                TotalFiles = totalFiles
                            });

                            extracted = true;
                        }
                        catch
                        {
                            extracted = false;
                        }
                    }

                    if (!extracted)
                    {
                        using var archive = ArchiveFactory.OpenArchive(archivePath, new ReaderOptions());
                        var nonDirEntries = archive.Entries.Where(e => !e.IsDirectory).ToList();
                        long totalBytes = nonDirEntries.Sum(e => e.Size);
                        int totalFiles = nonDirEntries.Count;
                        long bytesExtracted = 0;
                        int filesExtracted = 0;
                        DateTime lastReport = DateTime.UtcNow;
                        byte[] buffer = new byte[1024 * 512];

                        progress?.Report(new FolderProgressReport
                        {
                            ItemName = archiveFile.Name,
                            BytesProcessed = 0,
                            TotalBytes = totalBytes,
                            FilesProcessed = 0,
                            TotalFiles = totalFiles
                        });

                        foreach (var entry in nonDirEntries)
                        {
                            ct.ThrowIfCancellationRequested();
                            string entryTargetPath = Path.Combine(destDir, entry.Key ?? entry.ToString() ?? "");
                            string? dir = Path.GetDirectoryName(entryTargetPath);
                            if (!string.IsNullOrEmpty(dir)) Directory.CreateDirectory(dir);

                            using (var entryStream = entry.OpenEntryStream())
                            using (var outFile = new FileStream(entryTargetPath, FileMode.Create, FileAccess.Write, FileShare.None, 1024 * 512, FileOptions.SequentialScan))
                            {
                                int read;
                                while ((read = entryStream.Read(buffer, 0, buffer.Length)) > 0)
                                {
                                    outFile.Write(buffer, 0, read);
                                    bytesExtracted += read;

                                    var now = DateTime.UtcNow;
                                    if ((now - lastReport).TotalMilliseconds >= 40)
                                    {
                                        lastReport = now;
                                        progress?.Report(new FolderProgressReport
                                        {
                                            ItemName = archiveFile.Name,
                                            BytesProcessed = bytesExtracted,
                                            TotalBytes = totalBytes,
                                            FilesProcessed = filesExtracted,
                                            TotalFiles = totalFiles,
                                            CurrentFileName = entry.Key
                                        });
                                    }
                                }
                            }

                            if (keepMtime && entry.LastModifiedTime.HasValue)
                            {
                                try { File.SetLastWriteTime(entryTargetPath, entry.LastModifiedTime.Value); } catch { }
                            }
                            filesExtracted++;
                        }

                        progress?.Report(new FolderProgressReport
                        {
                            ItemName = archiveFile.Name,
                            BytesProcessed = totalBytes,
                            TotalBytes = totalBytes,
                            FilesProcessed = totalFiles,
                            TotalFiles = totalFiles
                        });
                    }

                    if (keepMtime)
                    {
                        try
                        {
                            Directory.SetLastWriteTime(destDir, originalTime);
                        }
                        catch { }
                    }

                    if (deleteOriginal)
                    {
                        try
                        {
                            SafeDeleteFile(archivePath);
                        }
                        catch (Exception delEx)
                        {
                            return (true, $"Unzipped: {archiveFile.Name} (Warning: Could not delete original archive: {delEx.Message})");
                        }
                    }

                    return (true, $"Unzipped: {archiveFile.Name}");
                }
                catch (Exception ex)
                {
                    return (false, $"Error unzipping {Path.GetFileName(archivePath)}: {ex.Message}");
                }
            }, ct);
        }

        public (long totalBytes, int totalFiles) PreScanItemForZip(string sourcePath)
        {
            try
            {
                if (Directory.Exists(sourcePath))
                {
                    var dirInfo = new DirectoryInfo(sourcePath);
                    long bytes = 0;
                    int count = 0;
                    foreach (var file in dirInfo.EnumerateFiles("*", SearchOption.AllDirectories))
                    {
                        bytes += file.Length;
                        count++;
                    }
                    return (bytes, count);
                }
                else if (File.Exists(sourcePath))
                {
                    var fileInfo = new FileInfo(sourcePath);
                    return (fileInfo.Length, 1);
                }
            }
            catch
            {
            }
            return (0, 0);
        }

        public (long totalBytes, int totalFiles) PreScanArchiveForUnzip(string archivePath)
        {
            try
            {
                if (!File.Exists(archivePath)) return (0, 0);
                var ext = Path.GetExtension(archivePath).ToLowerInvariant();
                if (ext == ".zip" || ext == ".osk" || ext == ".osz")
                {
                    using var zipStream = new FileStream(archivePath, FileMode.Open, FileAccess.Read, FileShare.ReadWrite, 1024 * 32, useAsync: false);
                    using var archive = new ZipArchive(zipStream, ZipArchiveMode.Read);
                    long bytes = 0;
                    int count = 0;
                    foreach (var entry in archive.Entries)
                    {
                        if (!string.IsNullOrEmpty(entry.Name))
                        {
                            bytes += entry.Length;
                            count++;
                        }
                    }
                    return (bytes, count);
                }
                else
                {
                    using var archive = ArchiveFactory.OpenArchive(archivePath, new ReaderOptions());
                    var nonDirEntries = archive.Entries.Where(e => !e.IsDirectory);
                    long bytes = 0;
                    int count = 0;
                    foreach (var entry in nonDirEntries)
                    {
                        bytes += entry.Size;
                        count++;
                    }
                    return (bytes, count);
                }
            }
            catch
            {
                try
                {
                    var fi = new FileInfo(archivePath);
                    return (fi.Length, 1);
                }
                catch
                {
                    return (0, 0);
                }
            }
        }

        private static void SafeDeleteDirectory(string directoryPath, int maxRetries = 5, int delayMs = 150)
        {
            if (!Directory.Exists(directoryPath)) return;

            for (int attempt = 1; attempt <= maxRetries; attempt++)
            {
                try
                {
                    var dirInfo = new DirectoryInfo(directoryPath);
                    foreach (var info in dirInfo.EnumerateFileSystemInfos("*", SearchOption.AllDirectories))
                    {
                        try
                        {
                            if (info.Attributes != FileAttributes.Normal)
                            {
                                info.Attributes = FileAttributes.Normal;
                            }
                        }
                        catch { }
                    }

                    if (dirInfo.Attributes != FileAttributes.Normal)
                    {
                        dirInfo.Attributes = FileAttributes.Normal;
                    }

                    Directory.Delete(directoryPath, recursive: true);
                    return;
                }
                catch (Exception) when (attempt < maxRetries)
                {
                    Thread.Sleep(delayMs);
                }
            }

            Directory.Delete(directoryPath, recursive: true);
        }

        private static void SafeDeleteFile(string filePath, int maxRetries = 5, int delayMs = 150)
        {
            if (!File.Exists(filePath)) return;

            for (int attempt = 1; attempt <= maxRetries; attempt++)
            {
                try
                {
                    var attr = File.GetAttributes(filePath);
                    if (attr != FileAttributes.Normal)
                    {
                        File.SetAttributes(filePath, FileAttributes.Normal);
                    }

                    File.Delete(filePath);
                    return;
                }
                catch (Exception) when (attempt < maxRetries)
                {
                    Thread.Sleep(delayMs);
                }
            }

            File.Delete(filePath);
        }
    }
}
