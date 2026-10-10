using System;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Media;
using System.Windows.Resources;

namespace ArchiveHelper.Services
{
    /// <summary>
    /// Icon geometry loaded from the standalone .svg files in <c>Assets/svg</c>.
    /// Those files are the single source of truth for every icon shape, so a shape
    /// can be re-designed in any SVG editor and restyled without touching XAML or C#.
    /// WPF has no native SVG renderer, so each file's <c>&lt;path d="..."&gt;</c>
    /// attributes are parsed into a <see cref="Geometry"/> once and cached for the
    /// lifetime of the process.
    /// </summary>
    public static class SvgAssets
    {
        // Matches the d attribute of any <path> element. The leading \s ensures we
        // don't match the tail of an attribute such as id="...".
        private static readonly Regex PathDataPattern = new Regex(
            "<path[^>]*?\\sd=\"([^\"]+)\"",
            RegexOptions.Compiled | RegexOptions.IgnoreCase
        );

        public static Geometry CheckMark { get; } = Load("checkmark");
        public static Geometry CheckMarkComplete { get; } = Load("check-complete");
        public static Geometry CheckMarkCompleteSmall { get; } = Load("check-complete-small");
        public static Geometry FolderSearch { get; } = Load("folder-search");
        public static Geometry Exit { get; } = Load("exit");
        public static Geometry Shield { get; } = Load("shield");
        public static Geometry ShieldAlert { get; } = Load("shield-alert");
        public static Geometry Thumbtack { get; } = Load("thumbtack");
        public static Geometry ThumbtackOff { get; } = Load("thumbtack-off");

        private static Geometry Load(string fileName)
        {
            var relativePath = $"Assets/svg/{fileName}.svg";
            var uri = new Uri($"pack://application:,,,/{relativePath}", UriKind.Absolute);

            StreamResourceInfo resource = Application.GetResourceStream(uri)
                ?? throw new FileNotFoundException(
                    $"SVG asset '{relativePath}' is missing. Ensure it is listed as a " +
                    "<Resource> item in the .csproj and that the project name has no spaces.");

            using (Stream stream = resource.Stream)
            using (var reader = new StreamReader(stream, Encoding.UTF8))
            {
                string svg = reader.ReadToEnd();

                var figures = new StringBuilder();
                foreach (Match match in PathDataPattern.Matches(svg))
                {
                    figures.Append(match.Groups[1].Value).Append(' ');
                }

                if (figures.Length == 0)
                {
                    throw new InvalidDataException($"SVG asset '{relativePath}' has no <path d=\"...\"> element.");
                }

                return Geometry.Parse(figures.ToString().Trim());
            }
        }
    }
}