using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Windows.Threading;
using ArchiveHelper.Models;
using ArchiveHelper.Services;
using Microsoft.Win32;

namespace ArchiveHelper
{
    public partial class MainWindow : Window
    {
        private readonly ArchiveService _archiveService = new();
        private static readonly Color ColorCardNormal = (Color)ColorConverter.ConvertFromString("#383838");
        private static readonly Color ColorCardZipHover = (Color)ColorConverter.ConvertFromString("#007ACC");
        private static readonly Color ColorCardUnzipHover = (Color)ColorConverter.ConvertFromString("#28A745");

        private static readonly Color ColorFolderNormal = (Color)ColorConverter.ConvertFromString("#3E3E3E");
        private static readonly Color ColorFolderZipHover = (Color)ColorConverter.ConvertFromString("#007ACC");
        private static readonly Color ColorFolderUnzipHover = (Color)ColorConverter.ConvertFromString("#28A745");

        private static readonly Color ColorFolderBgNormal = (Color)ColorConverter.ConvertFromString("#262626");
        private static readonly Color ColorFolderBgHover = (Color)ColorConverter.ConvertFromString("#333333");

        private readonly AppSettings _settings = AppSettings.Load();
        private bool _isBusy;
        private bool _elevationFailedOnStartup;
        private readonly ObservableCollection<OperationQueueItem> _queueItems = new();
        private DispatcherTimer? _popupCloseTimer;
        private bool _isQueuePinned;

        public MainWindow()
        {
            InitializeComponent();
            icQueueItems.ItemsSource = _queueItems;
            LoadSettings();
            UpdateAdminState();

            PreviewMouseDown += (s, e) =>
            {
                if (_isQueuePinned && !popupQueue.IsMouseOver && !globalProgressContainer.IsMouseOver)
                {
                    _isQueuePinned = false;
                    ScheduleCloseQueuePopup();
                }
            };

            KeyDown += (s, e) =>
            {
                if (e.Key == System.Windows.Input.Key.F6)
                {
                    SimulateDemoQueue();
                }
            };

            Loaded += (s, e) =>
            {
                CheckAutoElevationOnStartup();

                if (IsRunningAsAdmin())
                {
                    SetupIntegrityLevelDragDrop();
                }

                chkDeleteZip.Checked += (s, e) => SaveSettings();
                chkDeleteZip.Unchecked += (s, e) => SaveSettings();
                chkFastZip.Checked += (s, e) => SaveSettings();
                chkFastZip.Unchecked += (s, e) => SaveSettings();
                chkPredictExt.Checked += (s, e) => SaveSettings();
                chkPredictExt.Unchecked += (s, e) => SaveSettings();
                chkKeepMtimeZip.Checked += (s, e) => SaveSettings();
                chkKeepMtimeZip.Unchecked += (s, e) => SaveSettings();
                chkSingleFolderZip.Checked += (s, e) => SaveSettings();
                chkSingleFolderZip.Unchecked += (s, e) => SaveSettings();

                chkDeleteUnzip.Checked += (s, e) => SaveSettings();
                chkDeleteUnzip.Unchecked += (s, e) => SaveSettings();
                chkFastUnzip.Checked += (s, e) => SaveSettings();
                chkFastUnzip.Unchecked += (s, e) => SaveSettings();
                chkKeepTopDir.Checked += (s, e) => SaveSettings();
                chkKeepTopDir.Unchecked += (s, e) => SaveSettings();
                chkKeepMtimeUnzip.Checked += (s, e) => SaveSettings();
                chkKeepMtimeUnzip.Unchecked += (s, e) => SaveSettings();
            };

            Closing += (s, e) => SaveSettings();
        }

        [DllImport("dwmapi.dll")]
        private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int attrValue, int attrSize);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool ChangeWindowMessageFilter(uint message, uint dwFlag);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool ChangeWindowMessageFilterEx(IntPtr hWnd, uint msg, uint action, IntPtr changeInfo);

        [DllImport("shell32.dll")]
        private static extern void DragAcceptFiles(IntPtr hwnd, bool fAccept);

        [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
        private static extern uint DragQueryFile(IntPtr hDrop, uint iFile, System.Text.StringBuilder? lpszFile, uint cch);

        [DllImport("shell32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool DragQueryPoint(IntPtr hDrop, out POINT lppt);

        [DllImport("shell32.dll")]
        private static extern void DragFinish(IntPtr hDrop);

        [DllImport("ole32.dll")]
        private static extern int RevokeDragDrop(IntPtr hwnd);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool ClientToScreen(IntPtr hWnd, ref POINT lpPoint);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetCursorPos(out POINT lpPoint);

        [DllImport("user32.dll")]
        private static extern short GetAsyncKeyState(int vKey);

        [StructLayout(LayoutKind.Sequential)]
        private struct POINT
        {
            public int X;
            public int Y;
        }

        private const int WM_DROPFILES = 0x0233;
        private const uint WM_COPYDATA = 0x004A;
        private const uint WM_COPYGLOBALDATA = 0x0049;
        private const uint MSGFLT_ADD = 1;
        private const uint MSGFLT_ALLOW = 1;
        private const int VK_LBUTTON = 0x01;

        private const int DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1 = 19;
        private const int DWMWA_USE_IMMERSIVE_DARK_MODE = 20;
        private const int DWMWA_CAPTION_COLOR = 35;
        private const int DWMWA_TEXT_COLOR = 36;

        private bool _isHooked;
        private DispatcherTimer? _dragHighlightTimer;
        private FrameworkElement? _lastHighlightedElement;

        protected override void OnSourceInitialized(EventArgs e)
        {
            base.OnSourceInitialized(e);
            EnableDarkMode();
            SetupIntegrityLevelDragDrop();
        }

        private void SetupIntegrityLevelDragDrop()
        {
            var handle = new WindowInteropHelper(this).Handle;
            if (handle == IntPtr.Zero) return;

            ChangeWindowMessageFilter(WM_DROPFILES, MSGFLT_ADD);
            ChangeWindowMessageFilter(WM_COPYDATA, MSGFLT_ADD);
            ChangeWindowMessageFilter(WM_COPYGLOBALDATA, MSGFLT_ADD);

            ChangeWindowMessageFilterEx(handle, WM_DROPFILES, MSGFLT_ALLOW, IntPtr.Zero);
            ChangeWindowMessageFilterEx(handle, WM_COPYDATA, MSGFLT_ALLOW, IntPtr.Zero);
            ChangeWindowMessageFilterEx(handle, WM_COPYGLOBALDATA, MSGFLT_ALLOW, IntPtr.Zero);

            if (IsRunningAsAdmin())
            {
                RevokeDragDrop(handle);
                DragAcceptFiles(handle, true);

                if (!_isHooked)
                {
                    var source = HwndSource.FromHwnd(handle);
                    if (source != null)
                    {
                        source.AddHook(WndProc);
                        _isHooked = true;
                    }
                }

                StartDragHighlightTimer();
            }
        }

        private void StartDragHighlightTimer()
        {
            if (_dragHighlightTimer != null) return;
            _dragHighlightTimer = new DispatcherTimer(DispatcherPriority.Input)
            {
                Interval = TimeSpan.FromMilliseconds(40)
            };
            _dragHighlightTimer.Tick += DragHighlightTimer_Tick;
            _dragHighlightTimer.Start();
        }

        private void DragHighlightTimer_Tick(object? sender, EventArgs e)
        {
            if (!IsRunningAsAdmin()) return;

            bool isLButtonDown = (GetAsyncKeyState(VK_LBUTTON) & 0x8000) != 0;
            if (isLButtonDown)
            {
                if (!GetCursorPos(out POINT pt)) return;
                Point screenPt = new Point(pt.X, pt.Y);

                if (IsScreenPointInsideElement(btnFolderZip, screenPt))
                {
                    if (_lastHighlightedElement != btnFolderZip)
                    {
                        ClearDragHighlights();
                        _lastHighlightedElement = btnFolderZip;
                        AnimateBorderBrush(btnFolderZip, ColorFolderZipHover, 80);
                        AnimateBackground(btnFolderZip, ColorFolderBgHover, 80);
                    }
                }
                else if (IsScreenPointInsideElement(btnFolderUnzip, screenPt))
                {
                    if (_lastHighlightedElement != btnFolderUnzip)
                    {
                        ClearDragHighlights();
                        _lastHighlightedElement = btnFolderUnzip;
                        AnimateBorderBrush(btnFolderUnzip, ColorFolderUnzipHover, 80);
                        AnimateBackground(btnFolderUnzip, ColorFolderBgHover, 80);
                    }
                }
                else if (IsScreenPointInsideElement(dropZip, screenPt))
                {
                    if (_lastHighlightedElement != dropZip)
                    {
                        ClearDragHighlights();
                        _lastHighlightedElement = dropZip;
                        AnimateBorderBrush(dropZip, ColorCardZipHover, 80);
                    }
                }
                else if (IsScreenPointInsideElement(dropUnzip, screenPt))
                {
                    if (_lastHighlightedElement != dropUnzip)
                    {
                        ClearDragHighlights();
                        _lastHighlightedElement = dropUnzip;
                        AnimateBorderBrush(dropUnzip, ColorCardUnzipHover, 80);
                    }
                }
                else
                {
                    if (_lastHighlightedElement != null)
                    {
                        ClearDragHighlights();
                    }
                }
            }
            else
            {
                if (_lastHighlightedElement != null)
                {
                    ClearDragHighlights();
                }
            }
        }

        private void ClearDragHighlights()
        {
            if (_lastHighlightedElement == dropZip)
            {
                AnimateBorderBrush(dropZip, ColorCardNormal, 80);
            }
            else if (_lastHighlightedElement == dropUnzip)
            {
                AnimateBorderBrush(dropUnzip, ColorCardNormal, 80);
            }
            else if (_lastHighlightedElement == btnFolderZip)
            {
                AnimateBorderBrush(btnFolderZip, ColorFolderNormal, 80);
                AnimateBackground(btnFolderZip, ColorFolderBgNormal, 80);
            }
            else if (_lastHighlightedElement == btnFolderUnzip)
            {
                AnimateBorderBrush(btnFolderUnzip, ColorFolderNormal, 80);
                AnimateBackground(btnFolderUnzip, ColorFolderBgNormal, 80);
            }

            _lastHighlightedElement = null;
        }

        private IntPtr WndProc(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
        {
            if (msg == WM_DROPFILES)
            {
                HandleDropFiles(wParam);
                handled = true;
            }
            return IntPtr.Zero;
        }

        private static bool IsScreenPointInsideElement(FrameworkElement element, Point screenPoint)
        {
            if (!element.IsLoaded || !element.IsVisible) return false;
            try
            {
                Point localPt = element.PointFromScreen(screenPoint);
                return localPt.X >= 0 && localPt.X <= element.ActualWidth &&
                       localPt.Y >= 0 && localPt.Y <= element.ActualHeight;
            }
            catch
            {
                return false;
            }
        }

        private async void HandleDropFiles(IntPtr hDrop)
        {
            try
            {
                uint count = DragQueryFile(hDrop, 0xFFFFFFFF, null, 0);
                if (count == 0) return;

                var rawItems = new string[count];
                for (uint i = 0; i < count; i++)
                {
                    uint len = DragQueryFile(hDrop, i, null, 0);
                    var sb = new System.Text.StringBuilder((int)len + 2);
                    DragQueryFile(hDrop, i, sb, (uint)sb.Capacity);
                    rawItems[i] = sb.ToString();
                }

                if (_isBusy)
                {
                    return;
                }

                Point screenPt;
                if (GetCursorPos(out POINT cursorPt))
                {
                    screenPt = new Point(cursorPt.X, cursorPt.Y);
                }
                else
                {
                    DragQueryPoint(hDrop, out POINT clientPt);
                    var hwnd = new WindowInteropHelper(this).Handle;
                    ClientToScreen(hwnd, ref clientPt);
                    screenPt = new Point(clientPt.X, clientPt.Y);
                }

                ClearDragHighlights();

                if (IsScreenPointInsideElement(btnFolderZip, screenPt))
                {
                    string? selected = PromptFolderSelection("Select output destination folder for Zip");
                    if (!string.IsNullOrEmpty(selected))
                    {
                        await ProcessZipAsync(rawItems, selected);
                    }
                }
                else if (IsScreenPointInsideElement(btnFolderUnzip, screenPt))
                {
                    string? selected = PromptFolderSelection("Select destination folder for Unzip");
                    if (!string.IsNullOrEmpty(selected))
                    {
                        await ProcessUnzipAsync(rawItems, selected);
                    }
                }
                else if (IsScreenPointInsideElement(dropZip, screenPt))
                {
                    await ProcessZipAsync(rawItems, destinationDir: null);
                }
                else if (IsScreenPointInsideElement(dropUnzip, screenPt))
                {
                    await ProcessUnzipAsync(rawItems, destinationDir: null);
                }
                else
                {
                    Point winLocal = PointFromScreen(screenPt);
                    if (winLocal.X < ActualWidth / 2.0)
                    {
                        await ProcessZipAsync(rawItems, destinationDir: null);
                    }
                    else
                    {
                        await ProcessUnzipAsync(rawItems, destinationDir: null);
                    }
                }
            }
            catch (Exception)
            {
            }
            finally
            {
                DragFinish(hDrop);
            }
        }

        private void EnableDarkMode()
        {
            var handle = new WindowInteropHelper(this).Handle;
            if (handle == IntPtr.Zero) return;

            int trueValue = 1;
            if (DwmSetWindowAttribute(handle, DWMWA_USE_IMMERSIVE_DARK_MODE, ref trueValue, sizeof(int)) != 0)
            {
                DwmSetWindowAttribute(handle, DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1, ref trueValue, sizeof(int));
            }

            int captionColor = 0x00161616;
            DwmSetWindowAttribute(handle, DWMWA_CAPTION_COLOR, ref captionColor, sizeof(int));

            int textColor = 0x00FFFFFF;
            DwmSetWindowAttribute(handle, DWMWA_TEXT_COLOR, ref textColor, sizeof(int));
        }

        private void LoadSettings()
        {
            chkDeleteZip.IsChecked = _settings.DeleteZip;
            chkFastZip.IsChecked = _settings.FastZip;
            chkPredictExt.IsChecked = _settings.PredictExt;
            chkKeepMtimeZip.IsChecked = _settings.KeepMtimeZip;
            chkSingleFolderZip.IsChecked = _settings.SingleFolderZip;

            chkDeleteUnzip.IsChecked = _settings.DeleteUnzip;
            chkFastUnzip.IsChecked = _settings.FastUnzip;
            chkKeepTopDir.IsChecked = _settings.KeepTopDir;
            chkKeepMtimeUnzip.IsChecked = _settings.KeepMtimeUnzip;

            ApplyAlwaysOnTop(_settings.AlwaysOnTop);
            ApplyUnitMode(_settings.UseBinaryUnits);
        }

        private void SaveSettings()
        {
            _settings.DeleteZip = chkDeleteZip.IsChecked == true;
            _settings.FastZip = chkFastZip.IsChecked == true;
            _settings.PredictExt = chkPredictExt.IsChecked == true;
            _settings.KeepMtimeZip = chkKeepMtimeZip.IsChecked == true;
            _settings.SingleFolderZip = chkSingleFolderZip.IsChecked == true;

            _settings.DeleteUnzip = chkDeleteUnzip.IsChecked == true;
            _settings.FastUnzip = chkFastUnzip.IsChecked == true;
            _settings.KeepTopDir = chkKeepTopDir.IsChecked == true;
            _settings.KeepMtimeUnzip = chkKeepMtimeUnzip.IsChecked == true;
            _settings.AlwaysOnTop = Topmost;
            _settings.UseBinaryUnits = ProgressGeometryHelper.UseBinaryUnits;
            _settings.Save();
        }

        private static void AnimateBorderBrush(Border border, Color targetColor, double durationMs = 200)
        {
            if (border.BorderBrush is not SolidColorBrush scb || scb.IsFrozen)
            {
                scb = new SolidColorBrush(border.BorderBrush is SolidColorBrush cur ? cur.Color : targetColor);
                border.BorderBrush = scb;
            }

            var anim = new ColorAnimation
            {
                To = targetColor,
                Duration = TimeSpan.FromMilliseconds(durationMs)
            };
            scb.BeginAnimation(SolidColorBrush.ColorProperty, anim);
        }

        private static void AnimateBackground(Border border, Color targetColor, double durationMs = 200)
        {
            if (border.Background is not SolidColorBrush scb || scb.IsFrozen)
            {
                scb = new SolidColorBrush(border.Background is SolidColorBrush cur ? cur.Color : targetColor);
                border.Background = scb;
            }

            var anim = new ColorAnimation
            {
                To = targetColor,
                Duration = TimeSpan.FromMilliseconds(durationMs)
            };
            scb.BeginAnimation(SolidColorBrush.ColorProperty, anim);
        }

        private void Window_DragOver(object sender, DragEventArgs e)
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                e.Effects = DragDropEffects.Copy;
            }
            else
            {
                e.Effects = DragDropEffects.None;
            }
            e.Handled = true;
        }

        #region Utility Buttons (Admin, Always-on-Top, Exit)

        private static bool IsRunningAsAdmin()
        {
            using var identity = WindowsIdentity.GetCurrent();
            var principal = new WindowsPrincipal(identity);
            return principal.IsInRole(WindowsBuiltInRole.Administrator);
        }

        public bool ElevationFailedOnStartup
        {
            get => _elevationFailedOnStartup;
            set
            {
                _elevationFailedOnStartup = value;
                UpdateAdminState();
            }
        }

        private void UpdateAdminState(bool? forceAdminState = null)
        {
            bool isAdmin = forceAdminState ?? IsRunningAsAdmin();

            if (isAdmin)
            {
                iconAdmin.Data = SvgAssets.Shield;
                btnAdmin.IsEnabled = false;
                btnAdmin.ToolTip = "Running as Administrator";
                iconAdmin.ClearValue(System.Windows.Shapes.Shape.StrokeProperty);
                iconAdmin.Stroke = null;
                iconAdmin.StrokeThickness = 0;
                iconAdmin.Fill = Brushes.White;
            }
            else if (_elevationFailedOnStartup)
            {
                iconAdmin.Data = SvgAssets.ShieldAlert;
                btnAdmin.IsEnabled = true;
                btnAdmin.ToolTip = "Administrator elevation failed on startup (Click to retry)";
                iconAdmin.ClearValue(System.Windows.Shapes.Shape.StrokeProperty);
                iconAdmin.Stroke = null;
                iconAdmin.StrokeThickness = 0;
                iconAdmin.Fill = Brushes.White;
            }
            else
            {
                iconAdmin.Data = SvgAssets.Shield;
                btnAdmin.IsEnabled = true;
                btnAdmin.ToolTip = "Restart as Administrator";
                iconAdmin.Fill = Brushes.Transparent;
                iconAdmin.StrokeThickness = 20;
                iconAdmin.SetBinding(System.Windows.Shapes.Shape.StrokeProperty, new Binding("Foreground")
                {
                    Source = btnAdmin
                });
            }
        }

        internal void CheckAutoElevationOnStartup()
        {
            if (IsRunningAsAdmin())
            {
                if (!_settings.StartAsAdmin)
                {
                    _settings.StartAsAdmin = true;
                    SaveSettings();
                }
                return;
            }

            if (_settings.StartAsAdmin)
            {
                try
                {
                    var exePath = GetExecutablePath();
                    var workingDir = Path.GetDirectoryName(exePath);
                    if (string.IsNullOrEmpty(workingDir))
                    {
                        workingDir = AppContext.BaseDirectory;
                    }

                    var startInfo = new ProcessStartInfo
                    {
                        FileName = exePath,
                        UseShellExecute = true,
                        Verb = "runas",
                        WorkingDirectory = workingDir
                    };

                    ProcessStarter(startInfo);
                    Close();
                    Application.Current?.Shutdown();
                }
                catch (Exception)
                {
                    _elevationFailedOnStartup = true;
                    UpdateAdminState();
                }
            }
        }

        private static string GetExecutablePath()
        {
            // 1. Check AppContext.BaseDirectory (works in single-file published apps)
            var candidate1 = Path.Combine(AppContext.BaseDirectory, "ArchiveHelper.exe");
            if (File.Exists(candidate1)) return candidate1;

            // 2. Check executing assembly directory (works in dev/test/framework-dependent runners)
#pragma warning disable IL3000
            var asmLoc = typeof(MainWindow).Assembly.Location;
#pragma warning restore IL3000
            if (!string.IsNullOrEmpty(asmLoc))
            {
                var asmDir = Path.GetDirectoryName(asmLoc);
                if (!string.IsNullOrEmpty(asmDir))
                {
                    var candidate2 = Path.Combine(asmDir, "ArchiveHelper.exe");
                    if (File.Exists(candidate2)) return candidate2;
                }
            }

            // 3. Check Environment.ProcessPath
            var procPath = Environment.ProcessPath;
            if (!string.IsNullOrEmpty(procPath) && Path.GetFileName(procPath).Equals("ArchiveHelper.exe", StringComparison.OrdinalIgnoreCase))
            {
                return procPath;
            }

            // 4. Check MainModule
            try
            {
                var mainMod = Process.GetCurrentProcess().MainModule?.FileName;
                if (!string.IsNullOrEmpty(mainMod) && Path.GetFileName(mainMod).Equals("ArchiveHelper.exe", StringComparison.OrdinalIgnoreCase))
                {
                    return mainMod;
                }
            }
            catch { }

            return Path.Combine(AppContext.BaseDirectory, "ArchiveHelper.exe");
        }

        public Func<ProcessStartInfo, Process?> ProcessStarter { get; set; } = psi => Process.Start(psi);
        private bool _isElevating;

        private void BtnAdmin_Click(object sender, RoutedEventArgs e)
        {
            if (IsRunningAsAdmin() || !btnAdmin.IsEnabled || _isElevating) return;

            if (_isBusy)
            {
                return;
            }

            _isElevating = true;
            try
            {
                var exePath = GetExecutablePath();
                var workingDir = Path.GetDirectoryName(exePath);
                if (string.IsNullOrEmpty(workingDir))
                {
                    workingDir = AppContext.BaseDirectory;
                }

                var startInfo = new ProcessStartInfo
                {
                    FileName = exePath,
                    UseShellExecute = true,
                    Verb = "runas",
                    WorkingDirectory = workingDir
                };

                _settings.StartAsAdmin = true;
                SaveSettings();
                ProcessStarter(startInfo);
                Close();
                Application.Current?.Shutdown();
            }
            catch (System.ComponentModel.Win32Exception)
            {
                // User cancelled UAC prompt
                _isElevating = false;
            }
            catch (Exception)
            {
                _isElevating = false;
            }
        }

        private void ApplyAlwaysOnTop(bool enable)
        {
            Topmost = enable;
            _settings.AlwaysOnTop = enable;
            iconAlwaysOnTop.Opacity = enable ? 1.0 : 0.55;
            if (enable)
            {
                iconAlwaysOnTop.Data = SvgAssets.Thumbtack;
                btnAlwaysOnTop.ToolTip = "Always on Top: On (Click to disable)";
            }
            else
            {
                iconAlwaysOnTop.Data = SvgAssets.ThumbtackOff;
                btnAlwaysOnTop.ToolTip = "Always on Top: Off (Click to enable)";
            }
        }

        private void ApplyUnitMode(bool binary)
        {
            ProgressGeometryHelper.UseBinaryUnits = binary;
            _settings.UseBinaryUnits = binary;

            if (binary)
            {
                txtUnitTop.Text = "MiB";
                txtUnitBottom.Text = "GiB";
                btnUnitMode.ToolTip = "Unit: Binary (IEC - 1024: KiB, MiB, GiB)\nClick to switch to Decimal (1000)";
            }
            else
            {
                txtUnitTop.Text = "MB";
                txtUnitBottom.Text = "GB";
                btnUnitMode.ToolTip = "Unit: Decimal (SI - 1000: KB, MB, GB)\nClick to switch to Binary (1024)";
            }

            foreach (var item in _queueItems)
            {
                item.RefreshFormatting();
            }

            if (_queueItems.Count > 0)
            {
                UpdateGlobalProgress(_queueItems[0].IsZip);
            }
        }

        private void BtnUnitMode_Click(object sender, RoutedEventArgs e)
        {
            ApplyUnitMode(!ProgressGeometryHelper.UseBinaryUnits);
            SaveSettings();
        }

        private void BtnAlwaysOnTop_Click(object sender, RoutedEventArgs e)
        {
            ApplyAlwaysOnTop(!Topmost);
            SaveSettings();
        }

        private void BtnExit_Click(object sender, RoutedEventArgs e)
        {
            SaveSettings();
            Close();
            Application.Current?.Shutdown();
        }

        private void BtnClose_Click(object sender, RoutedEventArgs e)
        {
            BtnExit_Click(sender, e);
        }

        #endregion

        #region Operations Queue & Progress Popup

        private void ShowQueuePopup()
        {
            if (_queueItems.Count == 0 && !_isBusy) return;
            _popupCloseTimer?.Stop();

            if (popupQueue.Visibility != Visibility.Visible)
            {
                popupQueue.Visibility = Visibility.Visible;
                var anim = new DoubleAnimation
                {
                    From = popupQueue.Opacity,
                    To = 1.0,
                    Duration = TimeSpan.FromMilliseconds(160)
                };
                popupQueue.BeginAnimation(UIElement.OpacityProperty, anim);
            }
        }

        private void ScheduleCloseQueuePopup()
        {
            if (_isQueuePinned) return;
            _popupCloseTimer?.Stop();
            _popupCloseTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(250) };
            _popupCloseTimer.Tick += (s, e) =>
            {
                _popupCloseTimer.Stop();
                if (_isQueuePinned) return;
                var anim = new DoubleAnimation
                {
                    From = popupQueue.Opacity,
                    To = 0.0,
                    Duration = TimeSpan.FromMilliseconds(160)
                };
                anim.Completed += (s2, e2) =>
                {
                    if (popupQueue.Opacity <= 0.05)
                    {
                        popupQueue.Visibility = Visibility.Collapsed;
                    }
                };
                popupQueue.BeginAnimation(UIElement.OpacityProperty, anim);
            };
            _popupCloseTimer.Start();
        }

        private void GlobalProgress_MouseEnter(object sender, System.Windows.Input.MouseEventArgs e) => ShowQueuePopup();
        private void GlobalProgress_MouseLeave(object sender, System.Windows.Input.MouseEventArgs e) => ScheduleCloseQueuePopup();
        private void PopupQueue_MouseEnter(object sender, System.Windows.Input.MouseEventArgs e) => _popupCloseTimer?.Stop();
        private void PopupQueue_MouseLeave(object sender, System.Windows.Input.MouseEventArgs e) => ScheduleCloseQueuePopup();

        private void GlobalProgress_Click(object sender, System.Windows.Input.MouseButtonEventArgs e)
        {
            _isQueuePinned = !_isQueuePinned;
            if (_isQueuePinned)
            {
                ShowQueuePopup();
            }
            else
            {
                ScheduleCloseQueuePopup();
            }
        }

        private void UpdateGlobalProgress(bool isZip)
        {
            if (!Dispatcher.CheckAccess())
            {
                Dispatcher.BeginInvoke(() => UpdateGlobalProgress(isZip));
                return;
            }

            globalProgressContainer.Visibility = Visibility.Visible;

            if (_queueItems.Count == 0)
            {
                pathGlobalBytes.Data = null;
                pathGlobalFiles.Data = null;
                txtBottomSummaryBytes.Text = "";
                txtBottomSummaryFiles.Text = "";
                panelBottomSummary.Visibility = Visibility.Collapsed;
                return;
            }

            panelBottomSummary.Visibility = Visibility.Visible;
            pathGlobalBytes.Stroke = isZip
                ? new SolidColorBrush(Color.FromRgb(0, 122, 204))
                : new SolidColorBrush(Color.FromRgb(40, 167, 69));
            pathGlobalFiles.Stroke = new SolidColorBrush(Color.FromRgb(176, 38, 255));

            long totalBytesAll = 0;
            long processedBytesAll = 0;
            int totalFilesAll = 0;
            int processedFilesAll = 0;

            foreach (var item in _queueItems)
            {
                totalBytesAll += item.TotalBytes;
                processedBytesAll += item.BytesProcessed;
                totalFilesAll += item.TotalFiles;
                processedFilesAll += item.FilesProcessed;
            }

            double byteRatio = totalBytesAll > 0 ? Math.Clamp((double)processedBytesAll / totalBytesAll, 0.0, 1.0) : 0.0;
            double fileRatio = totalFilesAll > 0 ? Math.Clamp((double)processedFilesAll / totalFilesAll, 0.0, 1.0) : 0.0;

            pathGlobalBytes.Data = ProgressGeometryHelper.CreateConcentricArc(byteRatio, 22, 22, 19.8);
            pathGlobalFiles.Data = ProgressGeometryHelper.CreateConcentricArc(fileRatio, 22, 22, 15.7);

            if (totalBytesAll > 0)
            {
                txtBottomSummaryBytes.Text = $"{ProgressGeometryHelper.FormatBytes(processedBytesAll)} / {ProgressGeometryHelper.FormatBytes(totalBytesAll)}";
            }
            else
            {
                txtBottomSummaryBytes.Text = "";
            }

            if (totalFilesAll > 0)
            {
                txtBottomSummaryFiles.Text = $"({processedFilesAll}/{totalFilesAll})";
            }
            else
            {
                txtBottomSummaryFiles.Text = "";
            }
        }

        private void ShowGlobalCompletedAnimation(bool isZip)
        {
            if (!Dispatcher.CheckAccess())
            {
                Dispatcher.BeginInvoke(() => ShowGlobalCompletedAnimation(isZip));
                return;
            }

            gridGlobalCompleted.Visibility = Visibility.Visible;
            ellipseGlobalFill.Fill = isZip
                ? new SolidColorBrush(Color.FromRgb(0, 122, 204))
                : new SolidColorBrush(Color.FromRgb(40, 167, 69));

            // Only the highlights of the rings disappear; the background tracks stay visible!
            var fadeOutHighlights = new DoubleAnimation(canvasGlobalHighlights.Opacity, 0.0, TimeSpan.FromMilliseconds(250));
            var fadeInCompleted = new DoubleAnimation(gridGlobalCompleted.Opacity, 1.0, TimeSpan.FromMilliseconds(250));

            canvasGlobalHighlights.BeginAnimation(UIElement.OpacityProperty, fadeOutHighlights);
            gridGlobalCompleted.BeginAnimation(UIElement.OpacityProperty, fadeInCompleted);
        }

        private void ResetGlobalCompletedAnimation()
        {
            if (!Dispatcher.CheckAccess())
            {
                Dispatcher.BeginInvoke(ResetGlobalCompletedAnimation);
                return;
            }

            canvasGlobalHighlights.BeginAnimation(UIElement.OpacityProperty, null);
            gridGlobalCompleted.BeginAnimation(UIElement.OpacityProperty, null);
            canvasGlobalHighlights.Opacity = 1.0;
            gridGlobalCompleted.Opacity = 0.0;
            gridGlobalCompleted.Visibility = Visibility.Collapsed;
        }

        private async void SimulateDemoQueue()
        {
            if (_isBusy) return;
            _isBusy = true;
            ResetGlobalCompletedAnimation();
            _queueItems.Clear();

            var demoItems = new[]
            {
                new { Name = "asda", TotalBytes = 5368709120L, Files = 120, Speed = 95000000L },
                new { Name = "dfhdfg", TotalBytes = 3221225472L, Files = 85, Speed = 60000000L },
                new { Name = "twqe", TotalBytes = 1073741824L, Files = 30, Speed = 45000000L },
                new { Name = "zcxzcx", TotalBytes = 2147483648L, Files = 55, Speed = 50000000L },
                new { Name = "uy456u6", TotalBytes = 4294967296L, Files = 90, Speed = 75000000L }
            };

            foreach (var d in demoItems)
            {
                _queueItems.Add(new OperationQueueItem
                {
                    Name = d.Name,
                    IsZip = true,
                    TotalBytes = d.TotalBytes,
                    TotalFiles = d.Files,
                    Status = OperationItemStatus.Queued,
                    SizeText = $"{ProgressGeometryHelper.FormatBytes(0)} / {ProgressGeometryHelper.FormatBytes(d.TotalBytes)}",
                    FilesText = $"(0/{d.Files})"
                });
            }

            for (int i = 6; i <= 20; i++)
            {
                _queueItems.Add(new OperationQueueItem
                {
                    Name = $"batch_item_{i:D2}",
                    IsZip = true,
                    TotalBytes = 1024 * 1024 * 350,
                    TotalFiles = 15,
                    Status = OperationItemStatus.Queued,
                    SizeText = $"0 B / {ProgressGeometryHelper.FormatBytes(1024 * 1024 * 350)}",
                    FilesText = "(0/15)"
                });
            }

            txtCounter.Text = $"0/{_queueItems.Count}";
            UpdateGlobalProgress(isZip: true);
            ShowQueuePopup();

            try
            {
                for (int i = 0; i < demoItems.Length; i++)
                {
                    var item = _queueItems[i];
                    item.Status = OperationItemStatus.Processing;
                    long processed = 0;
                    int filesDone = 0;
                    long step = demoItems[i].Speed / 15;

                    while (processed < item.TotalBytes)
                    {
                        processed = Math.Min(item.TotalBytes, processed + step);
                        filesDone = (int)Math.Min((long)item.TotalFiles, (long)(item.TotalFiles * ((double)processed / item.TotalBytes)));
                        item.SetProgress(processed, item.TotalBytes, filesDone, item.TotalFiles);
                        txtCounter.Text = $"{i}/{_queueItems.Count}";
                        UpdateGlobalProgress(isZip: true);
                        await Task.Delay(40);
                    }
                    item.SetProgress(item.TotalBytes, item.TotalBytes, item.TotalFiles, item.TotalFiles);
                    item.Status = OperationItemStatus.Completed;
                    txtCounter.Text = $"{i + 1}/{_queueItems.Count}";
                    UpdateGlobalProgress(isZip: true);
                }

                txtCounter.Text = $"{demoItems.Length}/{_queueItems.Count}";
                UpdateGlobalProgress(isZip: true);
                ShowGlobalCompletedAnimation(isZip: true);
            }
            finally
            {
                _isBusy = false;
            }
        }

        #endregion

        #region Folder Selection Helper

        private string? PromptFolderSelection(string title)
        {
            var dialog = new OpenFolderDialog
            {
                Title = title,
                Multiselect = false
            };

            bool? result = dialog.ShowDialog(this);
            if (result == true && !string.IsNullOrWhiteSpace(dialog.FolderName))
            {
                return dialog.FolderName;
            }
            return null;
        }

        #endregion

        #region Zip Drop Area & Target Handlers

        private void DropZip_DragEnter(object sender, DragEventArgs e)
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                e.Effects = DragDropEffects.Copy;
                AnimateBorderBrush(dropZip, ColorCardZipHover);
            }
            else
            {
                e.Effects = DragDropEffects.None;
            }
            e.Handled = true;
        }

        private void DropZip_DragOver(object sender, DragEventArgs e)
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                e.Effects = DragDropEffects.Copy;
            }
            else
            {
                e.Effects = DragDropEffects.None;
            }
            e.Handled = true;
        }

        private void DropZip_DragLeave(object sender, DragEventArgs e)
        {
            AnimateBorderBrush(dropZip, ColorCardNormal);
        }

        private void BtnFolderZip_DragEnter(object sender, DragEventArgs e)
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                e.Effects = DragDropEffects.Copy;
                AnimateBorderBrush(btnFolderZip, ColorFolderZipHover);
                AnimateBackground(btnFolderZip, ColorFolderBgHover);
            }
            e.Handled = true;
        }

        private void BtnFolderZip_DragOver(object sender, DragEventArgs e)
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                e.Effects = DragDropEffects.Copy;
            }
            e.Handled = true;
        }

        private void BtnFolderZip_DragLeave(object sender, DragEventArgs e)
        {
            AnimateBorderBrush(btnFolderZip, ColorFolderNormal);
            AnimateBackground(btnFolderZip, ColorFolderBgNormal);
            e.Handled = true;
        }

        private async void BtnFolderZip_Drop(object sender, DragEventArgs e)
        {
            e.Handled = true;
            AnimateBorderBrush(btnFolderZip, ColorFolderNormal, 80);
            AnimateBackground(btnFolderZip, ColorFolderBgNormal, 80);
            AnimateBorderBrush(dropZip, ColorCardNormal, 80);

            if (!e.Data.GetDataPresent(DataFormats.FileDrop)) return;
            if (_isBusy) return;

            var rawItems = (string[])e.Data.GetData(DataFormats.FileDrop);
            if (rawItems == null || rawItems.Length == 0) return;

            string? selected = PromptFolderSelection("Select output destination folder for Zip");
            if (string.IsNullOrEmpty(selected))
            {
                return;
            }

            await ProcessZipAsync(rawItems, selected);
        }

        private async void DropZip_Drop(object sender, DragEventArgs e)
        {
            AnimateBorderBrush(dropZip, ColorCardNormal, 80);
            if (!e.Data.GetDataPresent(DataFormats.FileDrop)) return;
            if (_isBusy) return;

            var rawItems = (string[])e.Data.GetData(DataFormats.FileDrop);
            if (rawItems == null || rawItems.Length == 0) return;

            await ProcessZipAsync(rawItems, destinationDir: null);
        }

        private async Task ProcessZipAsync(string[] rawItems, string? destinationDir)
        {
            var validItems = rawItems.Where(p => Directory.Exists(p) || File.Exists(p)).ToArray();
            if (validItems.Length == 0) return;

            if (chkSingleFolderZip.IsChecked == true)
            {
                await ProcessSingleFolderZipAsync(validItems, destinationDir);
                return;
            }

            _isBusy = true;
            ResetGlobalCompletedAnimation();
            bool deleteOriginal = chkDeleteZip.IsChecked == true;
            bool isFast = chkFastZip.IsChecked == true;
            bool predictExt = chkPredictExt.IsChecked == true;
            bool keepMtime = chkKeepMtimeZip.IsChecked == true;
            int total = validItems.Length;
            int completed = 0;

            // Pre-scan all items upfront for strictly accurate batch progress
            var scanResults = new (string path, long bytes, int files)[validItems.Length];
            await Task.Run(() =>
            {
                Parallel.For(0, validItems.Length, i =>
                {
                    var scan = _archiveService.PreScanItemForZip(validItems[i]);
                    scanResults[i] = (validItems[i], scan.totalBytes, scan.totalFiles);
                });
            });

            var itemMap = new ConcurrentDictionary<string, OperationQueueItem>(StringComparer.OrdinalIgnoreCase);
            _queueItems.Clear();
            foreach (var (path, bytes, files) in scanResults)
            {
                var qItem = new OperationQueueItem
                {
                    Name = Path.GetFileName(path),
                    IsZip = true,
                    TotalBytes = bytes,
                    TotalFiles = files,
                    Status = OperationItemStatus.Queued,
                    SizeText = $"0 B / {ProgressGeometryHelper.FormatBytes(bytes)}",
                    FilesText = files > 0 ? $"(0/{files})" : ""
                };
                _queueItems.Add(qItem);
                itemMap[path] = qItem;
            }

            txtCounter.Text = $"0/{total}";
            UpdateGlobalProgress(isZip: true);

            try
            {
                if (isFast)
                {
                    await Parallel.ForEachAsync(validItems, new ParallelOptions { MaxDegreeOfParallelism = Environment.ProcessorCount }, async (item, ct) =>
                    {
                        itemMap.TryGetValue(item, out var qItem);
                        if (qItem != null)
                        {
                            Dispatcher.Invoke(() => qItem.Status = OperationItemStatus.Processing);
                        }

                        var progress = new DispatcherProgress<FolderProgressReport>(Dispatcher, report =>
                        {
                            if (qItem != null)
                            {
                                qItem.SetProgress(report.BytesProcessed, report.TotalBytes, report.FilesProcessed, report.TotalFiles);
                            }
                            UpdateGlobalProgress(isZip: true);
                        });

                        var (success, _) = await _archiveService.ZipItemAsync(item, deleteOriginal, predictExt, destinationDir, keepMtime, ct, progress);
                        int cur = Interlocked.Increment(ref completed);
                        Dispatcher.Invoke(() =>
                        {
                            if (qItem != null)
                            {
                                qItem.SetProgress(qItem.TotalBytes, qItem.TotalBytes, qItem.TotalFiles, qItem.TotalFiles);
                                qItem.Status = success ? OperationItemStatus.Completed : OperationItemStatus.Failed;
                            }
                            txtCounter.Text = $"{cur}/{total}";
                            UpdateGlobalProgress(isZip: true);
                        });
                    });
                }
                else
                {
                    foreach (var item in validItems)
                    {
                        itemMap.TryGetValue(item, out var qItem);
                        if (qItem != null)
                        {
                            qItem.Status = OperationItemStatus.Processing;
                        }

                        var progress = new DispatcherProgress<FolderProgressReport>(Dispatcher, report =>
                        {
                            if (qItem != null)
                            {
                                qItem.SetProgress(report.BytesProcessed, report.TotalBytes, report.FilesProcessed, report.TotalFiles);
                            }
                            UpdateGlobalProgress(isZip: true);
                        });

                        var (success, _) = await _archiveService.ZipItemAsync(item, deleteOriginal, predictExt, destinationDir, keepMtime, CancellationToken.None, progress);
                        if (qItem != null)
                        {
                            qItem.SetProgress(qItem.TotalBytes, qItem.TotalBytes, qItem.TotalFiles, qItem.TotalFiles);
                            qItem.Status = success ? OperationItemStatus.Completed : OperationItemStatus.Failed;
                        }
                        completed++;
                        txtCounter.Text = $"{completed}/{total}";
                        UpdateGlobalProgress(isZip: true);
                    }
                }

                txtCounter.Text = $"{completed}/{total}";
                UpdateGlobalProgress(isZip: true);
                ShowGlobalCompletedAnimation(isZip: true);
            }
            catch (Exception)
            {
            }
            finally
            {
                _isBusy = false;
            }
        }

        private async Task ProcessSingleFolderZipAsync(string[] items, string? destinationDir)
        {
            _isBusy = true;
            try
            {
                var dialog = new SingleFolderNameDialog(ArchiveService.GetZipSelectionName(items))
                {
                    Owner = this,
                    Topmost = Topmost
                };
                dialog.Resources.MergedDictionaries.Add(Resources);
                if (dialog.ShowDialog() != true) return;

                bool deleteOriginal = chkDeleteZip.IsChecked == true;
                bool predictExt = chkPredictExt.IsChecked == true;
                bool keepMtime = chkKeepMtimeZip.IsChecked == true;
                ResetGlobalCompletedAnimation();
                var queueItem = new OperationQueueItem
                {
                    Name = dialog.FolderName,
                    IsZip = true,
                    Status = OperationItemStatus.Processing
                };
                _queueItems.Clear();
                _queueItems.Add(queueItem);
                txtCounter.Text = "0/1";
                UpdateGlobalProgress(isZip: true);

                var progress = new DispatcherProgress<FolderProgressReport>(Dispatcher, report =>
                {
                    queueItem.SetProgress(report.BytesProcessed, report.TotalBytes, report.FilesProcessed, report.TotalFiles);
                    UpdateGlobalProgress(isZip: true);
                });
                var (success, message) = await _archiveService.ZipItemsAsync(
                    items, dialog.FolderName, deleteOriginal, predictExt, destinationDir, keepMtime,
                    progress: progress);
                queueItem.Status = success ? OperationItemStatus.Completed : OperationItemStatus.Failed;
                txtCounter.Text = "1/1";
                UpdateGlobalProgress(isZip: true);
                if (success)
                {
                    ShowGlobalCompletedAnimation(isZip: true);
                }
                if (!success || message.Contains("Warning:", StringComparison.Ordinal))
                {
                    MessageBox.Show(this, message, "Single zip", MessageBoxButton.OK, MessageBoxImage.Warning);
                }
            }
            finally
            {
                _isBusy = false;
            }
        }

        #endregion

        #region Unzip Drop Area & Target Handlers

        private void DropUnzip_DragEnter(object sender, DragEventArgs e)
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                e.Effects = DragDropEffects.Copy;
                AnimateBorderBrush(dropUnzip, ColorCardUnzipHover);
            }
            else
            {
                e.Effects = DragDropEffects.None;
            }
            e.Handled = true;
        }

        private void DropUnzip_DragOver(object sender, DragEventArgs e)
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                e.Effects = DragDropEffects.Copy;
            }
            else
            {
                e.Effects = DragDropEffects.None;
            }
            e.Handled = true;
        }

        private void DropUnzip_DragLeave(object sender, DragEventArgs e)
        {
            AnimateBorderBrush(dropUnzip, ColorCardNormal);
        }

        private void BtnFolderUnzip_DragEnter(object sender, DragEventArgs e)
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                e.Effects = DragDropEffects.Copy;
                AnimateBorderBrush(btnFolderUnzip, ColorFolderUnzipHover);
                AnimateBackground(btnFolderUnzip, ColorFolderBgHover);
            }
            e.Handled = true;
        }

        private void BtnFolderUnzip_DragOver(object sender, DragEventArgs e)
        {
            if (e.Data.GetDataPresent(DataFormats.FileDrop))
            {
                e.Effects = DragDropEffects.Copy;
            }
            e.Handled = true;
        }

        private void BtnFolderUnzip_DragLeave(object sender, DragEventArgs e)
        {
            AnimateBorderBrush(btnFolderUnzip, ColorFolderNormal);
            AnimateBackground(btnFolderUnzip, ColorFolderBgNormal);
            e.Handled = true;
        }

        private async void BtnFolderUnzip_Drop(object sender, DragEventArgs e)
        {
            e.Handled = true;
            AnimateBorderBrush(btnFolderUnzip, ColorFolderNormal, 80);
            AnimateBackground(btnFolderUnzip, ColorFolderBgNormal, 80);
            AnimateBorderBrush(dropUnzip, ColorCardNormal, 80);

            if (!e.Data.GetDataPresent(DataFormats.FileDrop)) return;
            if (_isBusy) return;

            var rawArchives = (string[])e.Data.GetData(DataFormats.FileDrop);
            if (rawArchives == null || rawArchives.Length == 0) return;

            string? selected = PromptFolderSelection("Select destination folder for Unzip");
            if (string.IsNullOrEmpty(selected))
            {
                return;
            }

            await ProcessUnzipAsync(rawArchives, selected);
        }

        private async void DropUnzip_Drop(object sender, DragEventArgs e)
        {
            AnimateBorderBrush(dropUnzip, ColorCardNormal, 80);
            if (!e.Data.GetDataPresent(DataFormats.FileDrop)) return;
            if (_isBusy) return;

            var rawArchives = (string[])e.Data.GetData(DataFormats.FileDrop);
            if (rawArchives == null || rawArchives.Length == 0) return;

            await ProcessUnzipAsync(rawArchives, destinationDir: null);
        }

        private async Task ProcessUnzipAsync(string[] rawArchives, string? destinationDir)
        {
            var validArchives = rawArchives.Where(File.Exists).ToArray();
            if (validArchives.Length == 0) return;

            _isBusy = true;
            ResetGlobalCompletedAnimation();
            bool deleteOriginal = chkDeleteUnzip.IsChecked == true;
            bool isFast = chkFastUnzip.IsChecked == true;
            bool keepTopDir = chkKeepTopDir.IsChecked == true;
            bool keepMtime = chkKeepMtimeUnzip.IsChecked == true;
            int total = validArchives.Length;
            int completed = 0;

            // Pre-scan all archives upfront for strictly accurate batch progress
            var scanResults = new (string path, long bytes, int files)[validArchives.Length];
            await Task.Run(() =>
            {
                Parallel.For(0, validArchives.Length, i =>
                {
                    var scan = _archiveService.PreScanArchiveForUnzip(validArchives[i]);
                    scanResults[i] = (validArchives[i], scan.totalBytes, scan.totalFiles);
                });
            });

            var archiveMap = new ConcurrentDictionary<string, OperationQueueItem>(StringComparer.OrdinalIgnoreCase);
            _queueItems.Clear();
            foreach (var (path, bytes, files) in scanResults)
            {
                var qItem = new OperationQueueItem
                {
                    Name = Path.GetFileName(path),
                    IsZip = false,
                    TotalBytes = bytes,
                    TotalFiles = files,
                    Status = OperationItemStatus.Queued,
                    SizeText = $"0 B / {ProgressGeometryHelper.FormatBytes(bytes)}",
                    FilesText = files > 0 ? $"(0/{files})" : ""
                };
                _queueItems.Add(qItem);
                archiveMap[path] = qItem;
            }

            txtCounter.Text = $"0/{total}";
            UpdateGlobalProgress(isZip: false);

            try
            {
                if (isFast)
                {
                    await Parallel.ForEachAsync(validArchives, new ParallelOptions { MaxDegreeOfParallelism = Environment.ProcessorCount }, async (archive, ct) =>
                    {
                        archiveMap.TryGetValue(archive, out var qItem);
                        if (qItem != null)
                        {
                            Dispatcher.Invoke(() => qItem.Status = OperationItemStatus.Processing);
                        }

                        var progress = new DispatcherProgress<FolderProgressReport>(Dispatcher, report =>
                        {
                            if (qItem != null)
                            {
                                qItem.SetProgress(report.BytesProcessed, report.TotalBytes, report.FilesProcessed, report.TotalFiles);
                            }
                            UpdateGlobalProgress(isZip: false);
                        });

                        var (success, _) = await _archiveService.UnzipArchiveAsync(archive, deleteOriginal, keepTopDir, destinationDir, keepMtime, ct, progress);
                        int cur = Interlocked.Increment(ref completed);
                        Dispatcher.Invoke(() =>
                        {
                            if (qItem != null)
                            {
                                qItem.SetProgress(qItem.TotalBytes, qItem.TotalBytes, qItem.TotalFiles, qItem.TotalFiles);
                                qItem.Status = success ? OperationItemStatus.Completed : OperationItemStatus.Failed;
                            }
                            txtCounter.Text = $"{cur}/{total}";
                            UpdateGlobalProgress(isZip: false);
                        });
                    });
                }
                else
                {
                    foreach (var archive in validArchives)
                    {
                        archiveMap.TryGetValue(archive, out var qItem);
                        if (qItem != null)
                        {
                            qItem.Status = OperationItemStatus.Processing;
                        }

                        var progress = new DispatcherProgress<FolderProgressReport>(Dispatcher, report =>
                        {
                            if (qItem != null)
                            {
                                qItem.SetProgress(report.BytesProcessed, report.TotalBytes, report.FilesProcessed, report.TotalFiles);
                            }
                            UpdateGlobalProgress(isZip: false);
                        });

                        var (success, _) = await _archiveService.UnzipArchiveAsync(archive, deleteOriginal, keepTopDir, destinationDir, keepMtime, CancellationToken.None, progress);
                        if (qItem != null)
                        {
                            qItem.SetProgress(qItem.TotalBytes, qItem.TotalBytes, qItem.TotalFiles, qItem.TotalFiles);
                            qItem.Status = success ? OperationItemStatus.Completed : OperationItemStatus.Failed;
                        }
                        completed++;
                        txtCounter.Text = $"{completed}/{total}";
                        UpdateGlobalProgress(isZip: false);
                    }
                }

                txtCounter.Text = $"{completed}/{total}";
                UpdateGlobalProgress(isZip: false);
                ShowGlobalCompletedAnimation(isZip: false);
            }
            catch (Exception)
            {
            }
            finally
            {
                _isBusy = false;
            }
        }

        #endregion

        private sealed class DispatcherProgress<T> : IProgress<T>
        {
            private readonly Dispatcher _dispatcher;
            private readonly Action<T> _handler;

            public DispatcherProgress(Dispatcher dispatcher, Action<T> handler)
            {
                _dispatcher = dispatcher;
                _handler = handler;
            }

            public void Report(T value)
            {
                if (_dispatcher.CheckAccess())
                {
                    _handler(value);
                }
                else
                {
                    _dispatcher.BeginInvoke(() =>
                    {
                        try
                        {
                            _handler(value);
                        }
                        catch
                        {
                        }
                    });
                }
            }
        }
    }
}
