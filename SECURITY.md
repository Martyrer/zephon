# Security policy

Zephon runs a UAC-elevated PowerShell script to add its firewall rule, writes to the HKCU
`Run` key, and parses data from devices on the local network. Please report problems in these
areas, or any other security issue, privately.

## Reporting a vulnerability

Use GitHub's private reporting: the **Security** tab of this repository, then
**Report a vulnerability**. Please do not open a public issue.

Include the Zephon version, the steps to reproduce, and what an attacker could do. You should
get a first answer within a week.

## Supported versions

Only the latest release gets security fixes while Zephon is in alpha.

## Design notes

- The elevated firewall script is started from the absolute Windows PowerShell path, and file
  paths reach it only base64-encoded, never as script text.
- Settings and logs stay in `%APPDATA%\Zephon`. The app sends nothing to the internet; it talks
  only to AirPlay receivers on the local network.

Problems in the Nyx-Refrain crates that Zephon depends on are also welcome here. They will be
passed on upstream.
