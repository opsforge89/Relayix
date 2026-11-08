import {readFileSync} from 'node:fs';
import {defineConfig} from 'vite';
const version=readFileSync(new URL('../Cargo.toml',import.meta.url),'utf8').match(/^version = "([^"]+)"/m)[1];
export default defineConfig({define:{__APP_VERSION__:JSON.stringify(version)}});
