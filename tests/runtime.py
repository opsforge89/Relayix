"""Shared ephemeral credentials. No hardcoded secrets or defaults."""
import json,os
from pathlib import Path
credentials=json.loads(Path(os.environ['RELAYIX_TEST_CREDENTIALS_FILE']).read_text())

version_file=Path(os.environ['RELAYIX_TEST_CREDENTIALS_FILE']).with_name('version.txt')
version=version_file.read_text().strip() if version_file.exists() else '0.8.2'
