# Security Policy

## Reporting

Report vulnerabilities privately to Mohamed.Hammad@SpacecraftSoftware.org,
or through GitHub's private vulnerability reporting on
`Spacecraft-Software/Vernier`. Do not open a public issue.

Expect an acknowledgement within 14 days. This is a hobby project; fixes
follow at hobby pace, with priority given to issues that let another local
user or a malicious display device affect the system.

## Scope

Vernier talks to the local display server (Wayland compositor, X server or
Mutter over the session bus) and reads profile files the user names. It opens
no network connections. In scope: crashes or misbehavior caused by malformed
profiles, malformed display or EDID data, or protocol messages.

## Supported versions

Only the latest release, and `main`, receive fixes.

## Disclosure

Coordinated: details are published with the fix, or after 90 days, whichever
comes first. Reporters are credited unless they ask not to be.
