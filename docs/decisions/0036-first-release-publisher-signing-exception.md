# 0036: Publisher signing exception for Silk 0.1.0

Status: Accepted, 2026-11-08

The maintainer explicitly authorized publishing the first release without a
Windows publisher certificate because recurring signing costs are unsuitable
for this free application. This supersedes the publisher-signing gate only for
the already tested 0.1.0 candidate built from commit
f98bb2f978090743875241436bda46c45b004d79. Future release policy remains unchanged.

The installer and app have no Authenticode publisher signature. Windows may
warn or block installation, and antivirus false positives remain possible.
Signing would not guarantee freedom from antivirus detections either.

Version-bound updater signatures, HTTPS transport, artifact hashes, data
preservation, and updater verification remain mandatory and enabled. This
exception does not authorize disabling Windows security or bypassing detections.

Publish the existing validated artifact without rebuilding or changing its bytes.
Release notes must disclose the missing publisher signature. The real local
upgrade, corruption rejection, version mismatch rejection, and preservation of
all six settings/catalog/clip hashes passed before publication.
