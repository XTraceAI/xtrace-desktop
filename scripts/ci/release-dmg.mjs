import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  checkDmgLayout,
  expectedLayout,
  inspectMountedLayout,
  withMountedImage,
} from './dmg-layout.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const IDENTITY = /^Developer ID Application: .+ \(([A-Z0-9]{10})\)$/;
// Finder sometimes saves the window before it applies the icon positions, so a
// disk image whose layout check fails is rebuilt this many times in total.
export const LAYOUT_ATTEMPTS = 3;

// Signing reads only these values. The App Store Connect key arrives as file
// contents and is written to a private file for Tauri and notarytool.
export function signingSettings(env) {
  const names = [
    'APPLE_CERTIFICATE',
    'APPLE_CERTIFICATE_PASSWORD',
    'APPLE_SIGNING_IDENTITY',
    'APPLE_API_KEY',
    'APPLE_API_ISSUER',
    'APPLE_API_PRIVATE_KEY',
  ];
  const missing = names.filter((name) => !env[name]);
  if (missing.length) throw new Error(`Signing needs ${missing.join(', ')}.`);
  const team = env.APPLE_SIGNING_IDENTITY.match(IDENTITY)?.[1];
  if (!team)
    throw new Error('APPLE_SIGNING_IDENTITY must be a "Developer ID Application" identity.');
  if (!/^[A-Z0-9]{10}$/.test(env.APPLE_API_KEY)) throw new Error('APPLE_API_KEY must be a key ID.');
  return {
    team,
    identity: env.APPLE_SIGNING_IDENTITY,
    certificate: env.APPLE_CERTIFICATE,
    certificatePassword: env.APPLE_CERTIFICATE_PASSWORD,
    keyId: env.APPLE_API_KEY,
    issuer: env.APPLE_API_ISSUER,
    privateKey: env.APPLE_API_PRIVATE_KEY,
  };
}

// Tauri signs, notarizes and staples the app when it sees these variables. It
// only warns when notarization settings are incomplete, so every other Apple
// variable is removed and the result is checked afterwards.
export function bundleEnvironment(inherited, settings, keyPath) {
  const env = { ...inherited, GH_TOKEN: '', GITHUB_TOKEN: '' };
  for (const name of Object.keys(env)) if (name.startsWith('APPLE_')) delete env[name];
  return {
    ...env,
    APPLE_CERTIFICATE: settings.certificate,
    APPLE_CERTIFICATE_PASSWORD: settings.certificatePassword,
    APPLE_SIGNING_IDENTITY: settings.identity,
    APPLE_API_KEY: settings.keyId,
    APPLE_API_ISSUER: settings.issuer,
    APPLE_API_KEY_PATH: keyPath,
    // Tauri skips the Finder layout script on CI unless told otherwise.
    TAURI_BUNDLER_DMG_IGNORE_CI: 'true',
  };
}

export function signatureProblems(details, team) {
  const problems = [];
  if (!new RegExp(`^TeamIdentifier=${team}$`, 'm').test(details))
    problems.push(`not signed by team ${team}`);
  if (!/^Authority=Developer ID Application: /m.test(details))
    problems.push('not signed with a Developer ID Application certificate');
  if (!/^Timestamp=/m.test(details)) problems.push('no secure timestamp');
  return problems;
}

export const notarizedByGatekeeper = (output) =>
  /: accepted$/m.test(output) && /^source=Notarized Developer ID$/m.test(output);

export function notarizationResult(output) {
  let result;
  try {
    result = JSON.parse(output);
  } catch {
    throw new Error('notarytool returned no readable result.');
  }
  if (!/^[0-9a-f-]{36}$/.test(result?.id ?? ''))
    throw new Error('notarytool returned no submission ID.');
  return { id: result.id, accepted: result.status === 'Accepted', status: result.status };
}

export async function packageDmg({
  env = process.env,
  repo = root,
  run = spawnSync,
  checkLayout = checkDmgLayout,
  mount = withMountedImage,
} = {}) {
  const settings = signingSettings(env);
  const config = JSON.parse(
    await readFile(join(repo, 'apps/desktop/src-tauri/tauri.conf.json'), 'utf8'),
  );
  const expected = await expectedLayout(join(repo, 'apps/desktop/src-tauri/tauri.conf.json'));
  const bundleDir = join(repo, 'target/release/bundle');
  const execute = (command, args, options = {}) => {
    const result = run(command, args, { cwd: repo, encoding: 'utf8', stdio: 'pipe', ...options });
    const output = `${result.stdout ?? ''}${result.stderr ?? ''}`;
    if (result.error || result.signal || result.status !== 0) {
      if (output.trim()) console.error(output.trim());
      throw new Error(`Packaging stopped: ${command} ${args[0]} failed.`);
    }
    return output;
  };
  const verifyApp = (app) => {
    execute('codesign', ['--verify', '--deep', '--strict', app]);
    const details = execute('codesign', ['-dvv', app]);
    const problems = signatureProblems(details, settings.team);
    if (!/flags=0x[0-9a-f]+\([^)]*\bruntime\b/.test(details))
      problems.push('built without the hardened runtime');
    if (problems.length) throw new Error(`The app is ${problems.join('; ')}.`);
    execute('xcrun', ['stapler', 'validate', app]);
    const assessment = execute('spctl', ['--assess', '--type', 'execute', '-vv', app]);
    if (!notarizedByGatekeeper(assessment))
      throw new Error(`Gatekeeper does not accept the app as notarized:\n${assessment.trim()}`);
  };

  const secrets = await mkdtemp(join(tmpdir(), 'xtrace-signing-'));
  try {
    const keyPath = join(secrets, `AuthKey_${settings.keyId}.p8`);
    await writeFile(keyPath, settings.privateKey, { mode: 0o600 });
    const bundleEnv = bundleEnvironment(env, settings, keyPath);
    const notaryAuth = ['--key', keyPath, '--key-id', settings.keyId, '--issuer', settings.issuer];

    const gatekeeper = run('spctl', ['--status'], { encoding: 'utf8' });
    console.log(
      `Gatekeeper on this runner: ${`${gatekeeper.stdout ?? ''}${gatekeeper.stderr ?? ''}`.trim()}`,
    );
    let image;
    for (let attempt = 1; !image; attempt += 1) {
      await rm(join(bundleDir, 'macos'), { recursive: true, force: true });
      await rm(join(bundleDir, 'dmg'), { recursive: true, force: true });
      execute('pnpm', ['tauri', 'bundle', '--bundles', 'app,dmg'], {
        env: bundleEnv,
        stdio: 'inherit',
      });
      verifyApp(join(bundleDir, 'macos', expected.app));
      const images = (await readdir(join(bundleDir, 'dmg'))).filter((name) =>
        name.endsWith('.dmg'),
      );
      if (images.length !== 1) throw new Error(`Expected one disk image, found ${images.length}.`);
      const candidate = join(bundleDir, 'dmg', images[0]);
      const { problems } = await checkLayout(candidate, expected);
      if (!problems.length) image = candidate;
      else {
        console.error(`Disk image layout attempt ${attempt}: ${problems.join(' ')}`);
        if (attempt >= LAYOUT_ATTEMPTS)
          throw new Error(`The disk image window layout was wrong in ${attempt} builds.`);
      }
    }

    const submission = notarizationResult(
      execute('xcrun', [
        'notarytool',
        'submit',
        image,
        ...notaryAuth,
        '--wait',
        '--timeout',
        '45m',
        '--output-format',
        'json',
      ]),
    );
    if (!submission.accepted) {
      console.error(execute('xcrun', ['notarytool', 'log', submission.id, ...notaryAuth]));
      throw new Error(`Apple did not accept the disk image (${submission.status}).`);
    }
    execute('xcrun', ['stapler', 'staple', image]);
    execute('xcrun', ['stapler', 'validate', image]);
    execute('codesign', ['--verify', '--strict', image]);
    const imageProblems = signatureProblems(execute('codesign', ['-dvv', image]), settings.team);
    if (imageProblems.length) throw new Error(`The disk image is ${imageProblems.join('; ')}.`);
    const assessment = execute('spctl', [
      '--assess',
      '--type',
      'open',
      '--context',
      'context:primary-signature',
      '-v',
      image,
    ]);
    if (!notarizedByGatekeeper(assessment))
      throw new Error(
        `Gatekeeper does not accept the disk image as notarized:\n${assessment.trim()}`,
      );
    execute('hdiutil', ['verify', image]);
    await mount(image, async (mountPoint) => {
      const { problems } = await inspectMountedLayout(mountPoint, expected);
      if (problems.length) throw new Error(`Final disk image layout: ${problems.join(' ')}`);
      verifyApp(join(mountPoint, expected.app));
    });

    const outputDir = join(repo, 'artifacts/release');
    const name = `${config.productName.replaceAll(' ', '-')}-${config.version}-macos-arm64.dmg`;
    await mkdir(outputDir, { recursive: true });
    await copyFile(image, join(outputDir, name));
    const digest = createHash('sha256')
      .update(await readFile(join(outputDir, name)))
      .digest('hex');
    await writeFile(join(outputDir, `${name}.sha256`), `${digest}  ${name}\n`);
    return { name, sha256: digest, notarization: submission.id };
  } finally {
    await rm(secrets, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.platform !== 'darwin' || process.argv.length !== 2) {
    console.error('Usage (macOS, with signing variables set): node scripts/ci/release-dmg.mjs');
    process.exitCode = 1;
  } else {
    packageDmg()
      .then((result) => console.log('Signed and notarized disk image: ' + JSON.stringify(result)))
      .catch((error) => {
        console.error(error.message);
        process.exitCode = 1;
      });
  }
}
