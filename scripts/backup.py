#!/usr/bin/env python3
"""Full password-encrypted backups using the active private deployment configuration."""
import argparse
import getpass
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import uuid
from deployment import ROOT, read_config, compose_environment, database_settings, CA_PATH


class BackupError(Exception):
    pass


class Deployment:
    def __init__(self):
        self.config = read_config()
        self.env, files = compose_environment(self.config)
        self.compose = ['docker', 'compose', '--project-directory', str(ROOT), '--env-file', os.devnull,
                        '--profile', self.env['DATABASE_MODE']]
        for path in files:
            self.compose += ['-f', str(path)]
        container = self.command(self.compose + ['ps', '-a', '-q', 'app']).decode().strip()
        if not container or '\n' in container:
            raise BackupError('Create the target app container before running backup operations')
        self.container = container
        info = json.loads(self.command(['docker', 'inspect', container]))[0]
        self.image = info['Image']
        self.running = info['State']['Running']
        self.network = next(iter(info['NetworkSettings']['Networks']))
        self.owned = set()
        db = database_settings(self.config)
        self.pg_env = {**self.env, 'PGHOST': db['host'], 'PGPORT': db['port'], 'PGUSER': db['user'],
                       'PGDATABASE': db['database'], 'PGPASSWORD': db['password'], 'PGSSLMODE': db['sslmode'],
                       'PGSSLROOTCERT': CA_PATH if db['ca'] else ('system' if db['sslmode'] == 'verify-full' else '')}
        self.ca = db['ca']

    def command(self, args, **kwargs):
        result = subprocess.run(args, env=kwargs.pop('env', self.env), stderr=subprocess.DEVNULL, stdout=subprocess.PIPE, **kwargs)
        if result.returncode:
            raise BackupError('Backup operation failed; no credentials or database output were printed')
        return result.stdout

    def docker(self, network):
        name = 'relayix-backup-' + uuid.uuid4().hex
        self.owned.add(name)
        return ['docker', 'run', '--rm', '--name', name, '-i', '--network', network]

    def pg(self, tool, *args):
        cmd = self.docker(self.network)
        for key in ('PGHOST', 'PGPORT', 'PGUSER', 'PGDATABASE', 'PGPASSWORD', 'PGSSLMODE', 'PGSSLROOTCERT'):
            cmd += ['-e', key]
        if self.ca:
            cmd += ['--mount', 'type=bind,source=' + self.ca + ',target=' + CA_PATH + ',readonly']
        return cmd + [self.config.get('POSTGRES_CLIENT_IMAGE', 'postgres:17.11-trixie'), tool, *args]

    def pg_run(self, tool, *args, **kwargs):
        result = subprocess.run(self.pg(tool, *args), env=self.pg_env, stderr=subprocess.DEVNULL, **kwargs)
        if result.returncode:
            raise BackupError('PostgreSQL backup operation failed; database output was suppressed')

    def cleanup(self):
        for name in self.owned:
            subprocess.run(['docker', 'rm', '-f', name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def password(args):
    if args.password_file:
        path = Path(args.password_file)
        if path.stat().st_mode & 0o077:
            raise BackupError('Password file must have owner-only permissions')
        value = path.read_text().rstrip('\r\n')
    else:
        value = getpass.getpass('Backup password: ')
        if args.operation == 'export' and value != getpass.getpass('Confirm backup password: '):
            raise BackupError('Passwords do not match')
    if not 12 <= len(value.encode()) <= 1024 or any(c in value for c in '\0\r\n'):
        raise BackupError('Use a backup password of 12–1024 UTF-8 bytes without line breaks')
    return value


def export_backup(deployment, path):
    if not deployment.running:
        raise BackupError('Start the source app before exporting a backup')
    if path.exists():
        raise BackupError('Output already exists; select a new backup filename')
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix='.relayix-backup-', dir=path.parent)
    try:
        with os.fdopen(descriptor, 'wb') as output:
            dump = subprocess.Popen(deployment.pg('pg_dump', '--format=custom', '--no-owner', '--no-acl'),
                                    env=deployment.pg_env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            try:
                result = subprocess.run(['docker', 'exec', '-i', '-e', 'RELAYIX_BACKUP_PASSWORD', deployment.container,
                                         '/usr/local/bin/relayix', 'backup-stream', 'seal'],
                                        env=deployment.env, stdin=dump.stdout, stdout=output, stderr=subprocess.DEVNULL)
                dump.stdout.close()
                if result.returncode or dump.wait():
                    raise BackupError('Export failed; incomplete encrypted output was discarded')
            finally:
                if dump.poll() is None:
                    dump.kill()
                    dump.wait()
            output.flush()
            os.fsync(output.fileno())
        # Never overwrite an existing archive, including one created during the export.
        os.link(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def read_exact(stream, size):
    result = b''
    while len(result) < size:
        part = stream.read(size - len(result))
        if not part:
            raise BackupError('Wrong password, damaged backup, or unsupported backup version')
        result += part
    return result


def verified_dump(deployment, path, output):
    cmd = deployment.docker('none') + ['--pull=never', '--entrypoint', '/usr/local/bin/relayix',
                                      '-e', 'RELAYIX_BACKUP_PASSWORD', deployment.image, 'backup-stream', 'open']
    with path.open('rb') as source:
        process = subprocess.Popen(cmd, env=deployment.env, stdin=source, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        try:
            size = struct.unpack('>I', read_exact(process.stdout, 4))[0]
            if not 0 < size <= 65536:
                raise BackupError('Invalid backup metadata')
            metadata = json.loads(read_exact(process.stdout, size))
            shutil.copyfileobj(process.stdout, output)
            if process.wait():
                raise BackupError('Backup verification failed; the database was not changed')
            output.seek(0)
            return metadata
        finally:
            process.stdout.close()
            if process.poll() is None:
                process.kill()
                process.wait()


# Revoke snapshot sessions and recover interrupted leases in the same restore transaction.
RESTORE_SQL = b"""
DELETE FROM sessions;
DELETE FROM login_attempts;
UPDATE delivery_attempts SET status='interrupted',finished_at=NOW(),error='Interrupted by backup restore' WHERE status='processing';
UPDATE deliveries SET status='queued',locked_at=NULL,next_attempt_at=GREATEST(NOW(),not_before) WHERE status='processing';
UPDATE alert_deliveries SET status='queued',locked_at=NULL,next_attempt_at=GREATEST(NOW(),not_before) WHERE status='processing';
INSERT INTO audit_log(actor,auth_method,action,target_type,affected) VALUES('local operator','local','backup.restore','database',1);
"""


def stage_settings(deployment, settings):
    path = Path(os.environ.get('RELAYIX_CONFIG_FILE', ROOT.parent / 'private/relayix-config/deployment.json')).resolve()
    # read_config already enforces the private path and string-only configuration.
    config = {**deployment.config, **settings}
    descriptor, temporary = tempfile.mkstemp(prefix='.relayix-recovered-settings-', dir=path.parent)
    with os.fdopen(descriptor, 'w') as output:
        json.dump(config, output, indent=2)
        output.write('\n')
        output.flush()
        os.fsync(output.fileno())
    return path, Path(temporary), config


def restore_backup(deployment, path, replace):
    # Both anonymous temporary files are owner-only and removed on close.
    with tempfile.TemporaryFile() as dump, tempfile.TemporaryFile() as sql:
        metadata = verified_dump(deployment, path, dump)
        target = int(deployment.command(deployment.pg('psql', '-XAtc', "SELECT current_setting('server_version_num')::int"),
                                        env=deployment.pg_env).strip())
        if target // 10000 < metadata['database_version'] // 10000:
            raise BackupError('Target PostgreSQL must be at least the backup source major version')
        count = deployment.command(deployment.pg('psql', '-XAtc', "SELECT COUNT(*) FROM pg_tables WHERE schemaname='public'"),
                                   env=deployment.pg_env).strip()
        if count != b'0' and not replace:
            raise BackupError('Target database is not empty; --replace is required to replace its Relayix data')
        deployment.pg_run('pg_restore', '--file=-', '--clean', '--if-exists', '--no-owner', '--no-acl', stdin=dump, stdout=sql)
        sql.write(RESTORE_SQL)
        sql.seek(0)
        config_path, recovery_path, config = stage_settings(deployment, metadata['settings'])
        restored = False
        try:
            if deployment.running:
                deployment.command(deployment.compose + ['stop', 'app'])
            deployment.pg_run('psql', '-X', '--single-transaction', '--set', 'ON_ERROR_STOP=1', stdin=sql, stdout=subprocess.DEVNULL)
            restored = True
            os.replace(recovery_path, config_path)
            deployment.env, _ = compose_environment(config)
            deployment.command(deployment.compose + ['up', '-d', '--no-deps', '--wait', 'app'])
        except BaseException:
            if not restored:
                recovery_path.unlink(missing_ok=True)
                if deployment.running:
                    deployment.command(deployment.compose + ['start', 'app'])
            else:
                # The staged file preserves the key if saving/restarting fails after DB commit.
                print('Database restored. Settings recovery path: ' + str(recovery_path if recovery_path.exists() else config_path), file=__import__('sys').stderr)
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['export', 'restore'])
    parser.add_argument('path', type=Path)
    parser.add_argument('--password-file', type=Path, help='Owner-only password file; otherwise prompt without echo')
    parser.add_argument('--replace', action='store_true', help='Explicitly replace the target Relayix database during restore')
    args = parser.parse_args()
    deployment = None
    try:
        value = password(args)
        deployment = Deployment()
        deployment.env['RELAYIX_BACKUP_PASSWORD'] = value
        deployment.pg_env['RELAYIX_BACKUP_PASSWORD'] = value
        if args.operation == 'export':
            export_backup(deployment, args.path.resolve())
        else:
            restore_backup(deployment, args.path.resolve(), args.replace)
        print('Encrypted backup ' + ('exported' if args.operation == 'export' else 'restored') + ' successfully')
    except BackupError as error:
        raise SystemExit(str(error)) from None
    except (OSError, ValueError, KeyError, StopIteration):
        raise SystemExit('Backup operation failed. Verify password, backup integrity, private configuration, disk space and Docker/PostgreSQL access. No secrets were printed.') from None
    finally:
        if deployment:
            deployment.cleanup()


if __name__ == '__main__':
    main()
