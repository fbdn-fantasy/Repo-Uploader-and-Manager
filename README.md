# Repo Uploader and Manager

A desktop application for Windows and Linux that makes creating, uploading, and managing GitHub repositories easier through a graphical interface.

Built with **Tauri 2, Rust, Svelte, and TypeScript**, the application uses system Git for repository operations and connects to GitHub through secure authentication.

## Features

- **GitHub Integration:** Sign in with your GitHub account and manage repositories without manually entering authentication tokens.
- **Multiple Accounts:** Connect and manage repositories across different GitHub accounts.
- **Create Repositories:** Create new GitHub repositories directly from the application, with optional README, license, and Git ignore templates.
- **Manage Existing Repositories:** Open existing local repositories or clone repositories from GitHub.
- **Automatic Repository Sync:** On startup and every 30 minutes, the app checks only repositories in your Repositories list that have an existing local checkout. It fetches changes from their GitHub origins and fast-forwards clean local branches. Repositories with local edits or diverging commits stay available for manual review. You can turn automatic syncing off in Settings.
- **File Management:** Add, modify, and remove repository files using your file manager.
- **Git Operations:** Stage individual files, create commits, push changes, pull updates, and fetch remote changes.
- **Branch Management:** Work with Git branches and manage branch deletion.
- **Repository Settings:** Rename repositories, update descriptions, change visibility, and manage repository associations.
- **Repository Deletion:** Remove local repositories, delete remote repositories, or manage them separately.
- **Security:** Credentials are stored securely using Windows Credential Manager or Linux Secret Service. The application also warns about potentially sensitive files before committing.

## Downloads and Installation

Download the latest release from the [GitHub Releases](../../releases) page.

### Windows

**Supported:** Windows 11 (64-bit)

| File | Description |
|---|---|
| `.exe` | Windows installer. |

**Requirements:** Git for Windows must be installed and available on the system PATH. The installer checks for Microsoft Edge WebView2 Runtime and can install it if missing.

### Linux

**Supported:** 64-bit Linux distributions.

| File | Description |
|---|---|
| `.rpm` | For Fedora and other compatible RPM-based distributions. |
| `.deb` | For Debian 13, Ubuntu 24.04, and newer compatible distributions. |
| `.tar.gz` | Portable archive for compatible Linux systems. Extract and run the application executable. |

The portable Linux version requires glibc 2.39 or newer and the necessary system libraries, including GTK 3 and WebKitGTK 4.1.

Linux installations also require Git and a compatible desktop credential store.

### Source Code

Source code ZIP archives are available alongside the installation packages for users who want to inspect, modify, or build the application themselves.

## Getting Started

1. Install the appropriate version for your operating system.
2. Launch Repo Uploader and Manager.
3. Select **Sign in with GitHub** and complete authentication in your browser.
4. Authorize the GitHub App for the accounts and repositories you want to manage.
5. Create a new repository, clone an existing one, or open a local repository to get started.

## License

Repo Uploader and Manager is open-source software licensed under the **GNU Affero General Public License v3.0 only**.

See [LICENSE](LICENSE) for the full license terms. Third-party attributions are available in [THIRD-PARTY.txt](THIRD-PARTY.txt).