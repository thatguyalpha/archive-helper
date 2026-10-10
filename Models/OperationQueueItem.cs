using System;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Media;

namespace ArchiveHelper.Models
{
    public enum OperationItemStatus
    {
        Queued,
        Processing,
        Completed,
        Failed
    }

    public static class ProgressGeometryHelper
    {
        public static Geometry CreateConcentricArc(double progress, double cx, double cy, double r)
        {
            var geom = new StreamGeometry();
            if (progress <= 0.001)
            {
                geom.Freeze();
                return geom;
            }

            using (var ctx = geom.Open())
            {
                Point startPoint = new Point(cx, cy - r);
                if (progress >= 0.999)
                {
                    Point bottomPoint = new Point(cx, cy + r);
                    ctx.BeginFigure(startPoint, isFilled: false, isClosed: true);
                    ctx.ArcTo(bottomPoint, new Size(r, r), 0, false, SweepDirection.Clockwise, isStroked: true, isSmoothJoin: true);
                    ctx.ArcTo(startPoint, new Size(r, r), 0, false, SweepDirection.Clockwise, isStroked: true, isSmoothJoin: true);
                }
                else
                {
                    double angle = Math.Clamp(progress * 359.99, 0.1, 359.99);
                    double rad = angle * Math.PI / 180.0;
                    Point endPoint = new Point(cx + r * Math.Sin(rad), cy - r * Math.Cos(rad));
                    bool isLargeArc = angle > 180.0;

                    ctx.BeginFigure(startPoint, isFilled: false, isClosed: false);
                    ctx.ArcTo(endPoint, new Size(r, r), 0, isLargeArc, SweepDirection.Clockwise, isStroked: true, isSmoothJoin: true);
                }
            }
            geom.Freeze();
            return geom;
        }

        public static bool UseBinaryUnits { get; set; } = true;

        public static string FormatBytes(long bytes)
        {
            return FormatBytes(bytes, UseBinaryUnits);
        }

        public static string FormatBytes(long bytes, bool binary)
        {
            if (bytes < 0) bytes = 0;
            if (binary)
            {
                if (bytes < 1024) return $"{bytes} B";
                if (bytes < 1024L * 1024) return $"{(bytes / 1024.0):F1} KiB";
                if (bytes < 1024L * 1024 * 1024) return $"{(bytes / (1024.0 * 1024.0)):F1} MiB";
                if (bytes < 1024L * 1024 * 1024 * 1024) return $"{(bytes / (1024.0 * 1024.0 * 1024.0)):F2} GiB";
                return $"{(bytes / (1024.0 * 1024.0 * 1024.0 * 1024.0)):F2} TiB";
            }
            else
            {
                if (bytes < 1000) return $"{bytes} B";
                if (bytes < 1000L * 1000) return $"{(bytes / 1000.0):F1} KB";
                if (bytes < 1000L * 1000 * 1000) return $"{(bytes / (1000.0 * 1000.0)):F1} MB";
                if (bytes < 1000L * 1000 * 1000 * 1000) return $"{(bytes / (1000.0 * 1000.0 * 1000.0)):F2} GB";
                return $"{(bytes / (1000.0 * 1000.0 * 1000.0 * 1000.0)):F2} TB";
            }
        }
    }

    public class OperationQueueItem : INotifyPropertyChanged
    {
        public static readonly Brush BrushZip = new SolidColorBrush((Color)ColorConverter.ConvertFromString("#007ACC"));
        public static readonly Brush BrushUnzip = new SolidColorBrush((Color)ColorConverter.ConvertFromString("#28A745"));
        public static readonly Brush BrushQueued = new SolidColorBrush((Color)ColorConverter.ConvertFromString("#555555"));
        public static readonly Brush BrushError = new SolidColorBrush((Color)ColorConverter.ConvertFromString("#DC3545"));
        public static readonly Brush BrushPurpleActive = new SolidColorBrush((Color)ColorConverter.ConvertFromString("#B026FF"));
        public static readonly Brush BrushPurpleQueued = new SolidColorBrush((Color)ColorConverter.ConvertFromString("#4A2866"));

        static OperationQueueItem()
        {
            BrushZip.Freeze();
            BrushUnzip.Freeze();
            BrushQueued.Freeze();
            BrushError.Freeze();
            BrushPurpleActive.Freeze();
            BrushPurpleQueued.Freeze();
        }

        private string _name = "";
        private bool _isZip;
        private long _bytesProcessed;
        private long _totalBytes;
        private int _filesProcessed;
        private int _totalFiles;
        private OperationItemStatus _status = OperationItemStatus.Queued;
        private string _sizeText = "Queued";
        private string _filesText = "";
        private Geometry _outerArcGeometry = Geometry.Empty;
        private Geometry _innerArcGeometry = Geometry.Empty;
        private Brush _arcBrush = BrushQueued;
        private Brush _innerBrush = BrushPurpleQueued;

        public event PropertyChangedEventHandler? PropertyChanged;

        public string Id { get; } = Guid.NewGuid().ToString();

        public string Name
        {
            get => _name;
            set => SetField(ref _name, value);
        }

        public bool IsZip
        {
            get => _isZip;
            set
            {
                if (SetField(ref _isZip, value))
                {
                    UpdateArcBrush();
                }
            }
        }

        public long BytesProcessed
        {
            get => _bytesProcessed;
            set
            {
                if (SetField(ref _bytesProcessed, value))
                {
                    RecalculateProgress();
                }
            }
        }

        public long TotalBytes
        {
            get => _totalBytes;
            set
            {
                if (SetField(ref _totalBytes, value))
                {
                    RecalculateProgress();
                }
            }
        }

        public int FilesProcessed
        {
            get => _filesProcessed;
            set
            {
                if (SetField(ref _filesProcessed, value))
                {
                    RecalculateProgress();
                }
            }
        }

        public int TotalFiles
        {
            get => _totalFiles;
            set
            {
                if (SetField(ref _totalFiles, value))
                {
                    RecalculateProgress();
                }
            }
        }

        public double BytesRatio => _totalBytes > 0 ? Math.Clamp((double)_bytesProcessed / _totalBytes, 0.0, 1.0) : 0.0;
        public double FilesRatio => _totalFiles > 0 ? Math.Clamp((double)_filesProcessed / _totalFiles, 0.0, 1.0) : 0.0;
        public double Progress => (_totalBytes > 0 && _totalFiles > 0) ? (BytesRatio + FilesRatio) / 2.0 : (_totalBytes > 0 ? BytesRatio : FilesRatio);

        public OperationItemStatus Status
        {
            get => _status;
            set
            {
                if (SetField(ref _status, value))
                {
                    UpdateArcBrush();
                    OnPropertyChanged(nameof(IsCompleted));
                    OnPropertyChanged(nameof(CompletedVisibility));
                }
            }
        }

        public bool IsCompleted => _status == OperationItemStatus.Completed;

        public string SizeText
        {
            get => _sizeText;
            set => SetField(ref _sizeText, value);
        }

        public string FilesText
        {
            get => _filesText;
            set => SetField(ref _filesText, value);
        }

        public Geometry OuterArcGeometry
        {
            get => _outerArcGeometry;
            private set => SetField(ref _outerArcGeometry, value);
        }

        public Geometry InnerArcGeometry
        {
            get => _innerArcGeometry;
            private set => SetField(ref _innerArcGeometry, value);
        }

        public Brush ArcBrush
        {
            get => _arcBrush;
            private set => SetField(ref _arcBrush, value);
        }

        public Brush InnerBrush
        {
            get => _innerBrush;
            private set => SetField(ref _innerBrush, value);
        }

        public Visibility CompletedVisibility => _status == OperationItemStatus.Completed ? Visibility.Visible : Visibility.Collapsed;
        public Brush FillBrush => _isZip ? BrushZip : BrushUnzip;

        public void SetProgress(long processed, long total, int filesDone, int filesTotal)
        {
            _bytesProcessed = processed;
            _totalBytes = total;
            _filesProcessed = filesDone;
            _totalFiles = filesTotal;

            RecalculateProgress();

            OnPropertyChanged(nameof(BytesProcessed));
            OnPropertyChanged(nameof(TotalBytes));
            OnPropertyChanged(nameof(FilesProcessed));
            OnPropertyChanged(nameof(TotalFiles));
        }

        public void RefreshFormatting()
        {
            RecalculateProgress();
        }

        private void RecalculateProgress()
        {
            // Per-operation concentric ring geometries: centered at (10, 10) in 20x20 canvas
            OuterArcGeometry = ProgressGeometryHelper.CreateConcentricArc(BytesRatio, 10, 10, 8.9);
            InnerArcGeometry = ProgressGeometryHelper.CreateConcentricArc(FilesRatio, 10, 10, 6.8);

            if (_totalBytes > 0)
            {
                SizeText = $"{ProgressGeometryHelper.FormatBytes(_bytesProcessed)} / {ProgressGeometryHelper.FormatBytes(_totalBytes)}";
            }
            else
            {
                SizeText = "Queued";
            }

            if (_totalFiles > 0)
            {
                FilesText = $"({_filesProcessed}/{_totalFiles})";
            }
            else
            {
                FilesText = "";
            }

            OnPropertyChanged(nameof(SizeText));
            OnPropertyChanged(nameof(FilesText));
            OnPropertyChanged(nameof(Progress));
        }

        private void UpdateArcBrush()
        {
            // Zip stays blue when completed; unzip stays green when completed
            ArcBrush = _status switch
            {
                OperationItemStatus.Completed => _isZip ? BrushZip : BrushUnzip,
                OperationItemStatus.Processing => _isZip ? BrushZip : BrushUnzip,
                OperationItemStatus.Failed => BrushError,
                _ => BrushQueued
            };

            InnerBrush = _status switch
            {
                OperationItemStatus.Completed => BrushPurpleActive,
                OperationItemStatus.Processing => BrushPurpleActive,
                OperationItemStatus.Failed => BrushError,
                _ => BrushPurpleQueued
            };

            OnPropertyChanged(nameof(CompletedVisibility));
            OnPropertyChanged(nameof(FillBrush));
        }

        protected bool SetField<T>(ref T field, T value, [CallerMemberName] string? propertyName = null)
        {
            if (System.Collections.Generic.EqualityComparer<T>.Default.Equals(field, value)) return false;
            field = value;
            OnPropertyChanged(propertyName);
            return true;
        }

        protected void OnPropertyChanged([CallerMemberName] string? propertyName = null)
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
        }
    }
}
