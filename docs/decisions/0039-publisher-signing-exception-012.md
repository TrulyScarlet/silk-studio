# 0039: Publisher signing exception for Silk 0.1.2

Status: Accepted, 2026-11-11

The maintainer explicitly authorized extending the existing publisher signing
exception to the 0.1.2 Intel encoder recovery bug fix. This version may be
published without an Authenticode publisher certificate. Windows may show an
unknown-publisher warning; release notes must disclose this limitation.

Version-bound updater signatures, HTTPS transport, artifact hashes, and updater
verification remain required. This exception changes no Windows security settings
and does not authorize unsigned future versions.
