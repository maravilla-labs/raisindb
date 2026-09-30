/**
 * Every test file runs with a throwaway HOME and cwd and none of the
 * RAISINDB_* connection variables, so no test can pick up the developer's real
 * `~/.raisinrc` (its server or its token) or a server from the shell.
 *
 * The cwd matters as much as HOME: loadConfig() searches for .raisinrc upward
 * from process.cwd() before falling back to HOME, and a checkout under the
 * home directory would find the real one on the way up.
 */
import fs from 'fs';
import os from 'os';
import path from 'path';

const home = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-cli-home-')));
process.env.HOME = home;
process.env.USERPROFILE = home;
for (const key of ['RAISINDB_SERVER', 'RAISINDB_TOKEN', 'RAISINDB_REPO']) {
  delete process.env[key];
}
process.chdir(home);
