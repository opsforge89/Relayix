#!/usr/bin/env python3
"""Rotate the active private deployment's master key without exposing either key."""
import argparse
import getpass
import os
from pathlib import Path
import subprocess
import sys
from backup import BackupError, Deployment, stage_settings
from deployment import CA_PATH, compose_environment


def native(deployment, old, new, mode):
    env = {**deployment.env, **deployment.app_environment, 'SECRETS_KEY_OLD': old, 'SECRETS_KEY': new}
    cmd = deployment.docker(deployment.network) + ['--pull=never', '--entrypoint', '/usr/local/bin/relayix']
    for name in deployment.app_environment.keys() | {'SECRETS_KEY', 'SECRETS_KEY_OLD'}:
        cmd += ['-e', name]
    if deployment.ca:
        cmd += ['--mount', 'type=bind,source=' + deployment.ca + ',target=' + CA_PATH + ',readonly']
    result = subprocess.run(cmd + [deployment.image, 'secrets-rotate', mode], env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if result.returncode:
        raise BackupError('Rotation validation or transaction failed; verify both keys and the database, then retry with the same proposed new key')


def rotate(deployment, new):
    old = deployment.config.get('SECRETS_KEY', '')
    if not old:
        raise BackupError('Enable stored credential encryption before rotating its key')
    # Validate the complete database in a rolled-back transaction while the app still runs.
    native(deployment, old, new, 'check')
    path, staged, config = stage_settings(deployment, {'SECRETS_KEY': new})
    stopped = False
    try:
        if deployment.running:
            deployment.command(deployment.compose + ['stop', 'app'])
        stopped = True
        native(deployment, old, new, 'apply')
        os.replace(staged, path)
        deployment.env, _ = compose_environment(config)
        deployment.command(deployment.compose + ['up', '-d', '--no-deps', '--wait', 'app'])
    except BaseException:
        if not stopped:
            staged.unlink(missing_ok=True)
        else:
            # A disconnect may hide a successful commit. Never restart with a possibly stale key.
            print('Application remains stopped or needs restart. Keep the proposed new key and private recovery settings: ' + str(staged if staged.exists() else path), file=sys.stderr)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--new-key-file', type=Path, help='Owner-only file containing the new base64/hex key; otherwise prompt without echo')
    args = parser.parse_args()
    deployment = None
    try:
        if args.new_key_file:
            if args.new_key_file.stat().st_mode & 0o077:
                raise BackupError('New-key file must have owner-only permissions')
            new = args.new_key_file.read_text().strip()
        else:
            new = getpass.getpass('New random 32-byte master key (base64 or hex): ').strip()
            if new != getpass.getpass('Confirm new master key: ').strip():
                raise BackupError('New keys do not match')
        if not new or len(new) > 128 or any(char in new for char in '\0\r\n'):
            raise BackupError('Provide a random 32-byte key encoded as base64 or hexadecimal')
        deployment = Deployment()
        rotate(deployment, new)
        print('Master key rotated atomically; update other replicas and private GitHub deployment secrets before restarting them')
    except BackupError as error:
        raise SystemExit(str(error)) from None
    except (OSError, ValueError, KeyError, StopIteration):
        raise SystemExit('Rotation failed; verify private configuration, Docker access and database permissions. No keys were printed.') from None
    finally:
        if deployment:
            deployment.cleanup()


if __name__ == '__main__':
    main()
