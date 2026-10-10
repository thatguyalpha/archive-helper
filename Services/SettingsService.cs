using System;
using System.IO;
using System.Text.Json;

namespace ArchiveHelper.Services
{
    public class AppSettings
    {
        public bool DeleteZip { get; set; } = true;
        public bool FastZip { get; set; } = false;
        public bool PredictExt { get; set; } = true;
        public bool KeepMtimeZip { get; set; } = true;
        public bool DeleteUnzip { get; set; } = true;
        public bool FastUnzip { get; set; } = false;
        public bool KeepTopDir { get; set; } = true;
        public bool KeepMtimeUnzip { get; set; } = true;
        public bool AlwaysOnTop { get; set; } = false;
        public bool StartAsAdmin { get; set; } = false;
        public bool UseBinaryUnits { get; set; } = true;

        private static readonly string SettingsFolder = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "ArchiveHelper"
        );

        private static readonly string SettingsFilePath = Path.Combine(SettingsFolder, "settings.json");

        public static AppSettings Load()
        {
            try
            {
                if (File.Exists(SettingsFilePath))
                {
                    string json = File.ReadAllText(SettingsFilePath);
                    var settings = JsonSerializer.Deserialize<AppSettings>(json);
                    if (settings != null)
                    {
                        return settings;
                    }
                }
            }
            catch
            {
                // Fallback to defaults on read error
            }

            return new AppSettings();
        }

        public void Save()
        {
            try
            {
                Directory.CreateDirectory(SettingsFolder);
                string json = JsonSerializer.Serialize(this, new JsonSerializerOptions { WriteIndented = true });
                File.WriteAllText(SettingsFilePath, json);
            }
            catch
            {
                // Suppress save errors to avoid interrupting user
            }
        }
    }
}
