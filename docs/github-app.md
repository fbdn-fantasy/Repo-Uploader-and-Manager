# GitHub App configuration

Repo Uploader and Manager uses the registered app with App ID `5849271` and public client ID `Iv23licw15FcmAEQUGOn`. Keep **Device Flow** and expiring user access tokens enabled in the app registration. The client uses the public client ID for both sign-in and refresh. The desktop build requires no server, callback URL, client secret or app private key. [Registered app](https://github.com/apps/repo-uploader-and-manager) · [GitHub Device Flow](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app#using-the-device-flow-to-generate-a-user-access-token)

## Sign in and install

1. Select **Sign in with GitHub**. Copy the temporary code from the app and enter it on GitHub's device page in the browser. Check the account shown by GitHub before approving. Cancelling in the desktop app stops that attempt locally. [GitHub device authorization](https://github.com/login/device)
2. Grant the app an installation on the account that owns the repositories. Choose the repositories to include. An organization owner may need to approve the installation or accept updated permissions. An organization using SAML SSO may require an active SSO session before reauthorization. [Install the GitHub App](https://github.com/apps/repo-uploader-and-manager/installations/new)
3. Select the active account in the application. Each local checkout is associated with one account. For reauthorization, choose the same GitHub account in the browser. A different account can be added separately.

The app discovers repositories through each accessible, active installation. A repository excluded from the installation may still be visible in a browser. Change the installation's repository selection before retrying in the desktop app. [Manage installed GitHub Apps](https://github.com/settings/installations)

## Local commits and the first push

The app uses your configured Git author name and email for commits. If either field is missing, it fills that field for the commit using the signed-in account’s username or its ID-based GitHub `noreply` address. This also works with accounts saved in V2.

If GitHub declines repository creation, the app opens the local checkout and displays the reason. To upload from that checkout, select **Push** and choose an existing private repository or create a new private repository. After connecting, select **Push** again to send your commits. A newly created repository defaults to **Push privately**.

## Permissions and endpoints

The desktop client uses GitHub App user access tokens with GitHub REST API version `2026-03-10`. Operations depend on both the user's authority and the installation's grants.

| Operation | Endpoint | Access |
| --- | --- | --- |
| Read signed-in identity | `GET /user` | Basic profile |
| Find accessible installations | `GET /user/installations` | No additional permission |
| List installed repositories | `GET /user/installations/{id}/repositories` | Metadata read |
| Inspect repository | `GET /repos/{owner}/{repo}` | Metadata read |
| Create empty private repository for current user | `POST /user/repos` | Repository creation write or Administration write |
| Update name, description or visibility | `PATCH /repos/{owner}/{repo}` | Administration write and sufficient user privileges |
| Delete remote repository | `DELETE /repos/{owner}/{repo}` | Administration write and sufficient user privileges |

Repository Contents read and write enables Git over HTTPS, subject to branch protection and organization policy. The app requests no organization or enterprise permission. GitHub may decline a push that would edit workflow files or that violates repository rules. [Repository endpoints](https://docs.github.com/en/rest/repos/repos) · [App installation endpoints](https://docs.github.com/en/rest/apps/installations) · [User endpoints](https://docs.github.com/en/rest/users/users#get-the-authenticated-user)

## Credential lifecycle

The device exchange requests expiring credentials. Before expiry, the backend sends the refresh token, client ID and refresh grant to GitHub. It replaces both tokens in one secure-store item associated with the numeric GitHub account ID. Reauthorize if the refresh token expires, authorization is revoked, or replacement credentials cannot be saved. [Refresh behavior for Device Flow tokens](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/refreshing-user-access-tokens)

On Windows the app stores this item in Windows Credential Manager for the current user. On Linux it uses Secret Service through the system's `secret-tool`. A running session bus and unlocked keyring are required. Debian/Ubuntu provide the utility in `libsecret-tools`. On Fedora it is provided by `libsecret`.

Signing out removes the local credential. To revoke authorization on GitHub, open GitHub settings, select **Applications**, then **Authorized GitHub Apps**. Installation removal is managed separately by the installation owner. [GitHub application settings](https://github.com/settings/apps/authorizations)
