using System.Windows;
using System.Windows.Controls;
using ArchiveHelper.Services;

namespace ArchiveHelper
{
    public partial class SingleFolderNameDialog : Window
    {
        public string FolderName => txtFolderName.Text;

        public SingleFolderNameDialog(string defaultName)
        {
            InitializeComponent();
            txtFolderName.Text = defaultName;
        }

        private void Window_ContentRendered(object sender, System.EventArgs e)
        {
            txtFolderName.Focus();
            txtFolderName.SelectAll();
        }

        private void FolderName_TextChanged(object sender, TextChangedEventArgs e)
        {
            if (txtValidation != null) txtValidation.Visibility = Visibility.Collapsed;
        }

        private void CreateZip_Click(object sender, RoutedEventArgs e)
        {
            if (!ArchiveService.IsValidZipFolderName(FolderName))
            {
                txtValidation.Text = "Enter a valid zip name without path separators or reserved characters.";
                txtValidation.Visibility = Visibility.Visible;
                txtFolderName.Focus();
                return;
            }
            DialogResult = true;
        }
    }
}
