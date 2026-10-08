import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { copyFile, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  LAYOUT_ATTEMPTS,
  bundleEnvironment,
  notarizationResult,
  notarizedByGatekeeper,
  packageDmg,
  signatureProblems,
  signingSettings,
} from './release-dmg.mjs';
import { dsStore, iloc } from './ds-store-fixture.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const env = {
  APPLE_CERTIFICATE: 'base64-p12',
  APPLE_CERTIFICATE_PASSWORD: 'p12-password',
  APPLE_SIGNING_IDENTITY: 'Developer ID Application: Example Inc. (ABCDE12345)',
  APPLE_API_KEY: 'KEY1234567',
  APPLE_API_ISSUER: '00000000-0000-0000-0000-000000000000',
  APPLE_API_PRIVATE_KEY: '-----BEGIN PRIVATE KEY-----\nexample\n-----END PRIVATE KEY-----\n',
};
const signed = [
  'Authority=Developer ID Application: Example Inc. (ABCDE12345)',
  'Authority=Developer ID Certification Authority',
  'Timestamp=Oct 7, 2026 at 9:30:51 PM',
  'TeamIdentifier=ABCDE12345',
  'CodeDirectory v=20500 size=1 flags=0x10000(runtime) hashes=1+3 location=embedded',
].join('\n');
const accepted =
  'X: accepted\nsource=Notarized Developer ID\norigin=Developer ID Application: Example';

test('signing needs every Apple value and a Developer ID Application identity', () => {
  assert.equal(signingSettings(env).team, 'ABCDE12345');
  assert.throws(
    () => signingSettings({ ...env, APPLE_API_PRIVATE_KEY: '' }),
    /APPLE_API_PRIVATE_KEY/,
  );
  assert.throws(
    () =>
      signingSettings({
        ...env,
        APPLE_SIGNING_IDENTITY: 'Apple Development: Someone (ABCDE12345)',
      }),
    /Developer ID Application/,
  );
});

test('the bundler sees only the chosen Apple values and runs the Finder layout script', () => {
  const settings = signingSettings(env);
  const bundleEnv = bundleEnvironment(
    {
      ...env,
      APPLE_ID: 'someone@example.com',
      APPLE_PASSWORD: 'x',
      GH_TOKEN: 'token',
      PATH: '/bin',
    },
    settings,
    '/private/key.p8',
  );
  assert.equal(bundleEnv.APPLE_ID, undefined);
  assert.equal(bundleEnv.APPLE_PASSWORD, undefined);
  assert.equal(bundleEnv.APPLE_API_PRIVATE_KEY, undefined);
  assert.equal(bundleEnv.APPLE_API_KEY_PATH, '/private/key.p8');
  assert.equal(bundleEnv.APPLE_SIGNING_IDENTITY, env.APPLE_SIGNING_IDENTITY);
  assert.equal(bundleEnv.TAURI_BUNDLER_DMG_IGNORE_CI, 'true');
  assert.equal(bundleEnv.GH_TOKEN, '');
  assert.equal(bundleEnv.PATH, '/bin');
});

test('signature, Gatekeeper and notarytool output are read strictly', () => {
  assert.deepEqual(signatureProblems(signed, 'ABCDE12345'), []);
  assert.deepEqual(signatureProblems('Signature=adhoc\nTeamIdentifier=not set', 'ABCDE12345'), [
    'not signed by team ABCDE12345',
    'not signed with a Developer ID Application certificate',
    'no secure timestamp',
  ]);
  assert.equal(notarizedByGatekeeper(accepted), true);
  assert.equal(notarizedByGatekeeper('X: accepted\nsource=Developer ID'), false);
  assert.equal(notarizedByGatekeeper('X: rejected\nsource=Notarized Developer ID'), false);
  const id = '91404c90-85e6-4fe7-8850-50174682ba36';
  assert.deepEqual(notarizationResult(JSON.stringify({ id, status: 'Accepted' })), {
    id,
    accepted: true,
    status: 'Accepted',
  });
  assert.equal(notarizationResult(JSON.stringify({ id, status: 'Invalid' })).accepted, false);
  assert.throws(() => notarizationResult('not json'), /no readable result/);
});

// A copy of the repository's Tauri settings plus fake Apple tools: `pnpm` writes
// a disk image, the layout check returns the queued results in order, and the
// final mount shows a volume laid out as tauri.conf.json asks. `tool` can replace
// any one tool's answer, and `finalIcons` the icons the final mount shows.
async function fakeRelease(
  t,
  {
    layouts,
    notarization = 'Accepted',
    tool = () => undefined,
    finalIcons = [iloc('Applications', 480, 170), iloc('XTrace Desktop.app', 180, 170)],
  },
) {
  const repo = await mkdtemp(join(tmpdir(), 'xtrace-release-dmg-test-'));
  t.after(() => rm(repo, { recursive: true, force: true }));
  const config = join(repo, 'apps/desktop/src-tauri/tauri.conf.json');
  await mkdir(dirname(config), { recursive: true });
  await copyFile(join(root, 'apps/desktop/src-tauri/tauri.conf.json'), config);
  const volume = join(repo, 'volume');
  await mkdir(join(volume, 'XTrace Desktop.app'), { recursive: true });
  await mkdir(join(volume, '.background'));
  await writeFile(join(volume, '.background/background.tiff'), '');
  await symlink('/Applications', join(volume, 'Applications'));
  await writeFile(join(volume, '.DS_Store'), dsStore(finalIcons));
  const calls = [];
  const keys = [];
  const run = (command, args, options) => {
    calls.push([command, ...args].join(' '));
    const replaced = tool(command, args);
    if (replaced) return replaced;
    if (command === 'pnpm') {
      keys.push(options.env.APPLE_API_KEY_PATH);
      assert.equal(readFileSync(options.env.APPLE_API_KEY_PATH, 'utf8'), env.APPLE_API_PRIVATE_KEY);
      const dmg = join(repo, 'target/release/bundle/dmg');
      mkdirSync(dmg, { recursive: true });
      writeFileSync(join(dmg, 'XTrace Desktop_0.1.0_aarch64.dmg'), 'disk image bytes');
      return { status: 0 };
    }
    if (command === 'codesign' && args[0] === '-dvv') return { status: 0, stderr: signed };
    if (command === 'spctl') return { status: 0, stderr: accepted };
    if (args[1] === 'submit')
      return {
        status: 0,
        stderr: 'Warning: an unrelated notice on stderr.\n',
        stdout: JSON.stringify({
          id: 'ebde0ca8-1cbf-4024-b482-76ad213a6222',
          status: notarization,
        }),
      };
    return { status: 0, stdout: '' };
  };
  const options = {
    env,
    repo,
    run,
    checkLayout: async () => ({ problems: layouts.shift() ?? [] }),
    mount: async (_image, inspect) => inspect(volume),
  };
  return { repo, calls, keys, options };
}

test('packaging rebuilds a misplaced layout, then notarizes, staples and names the image', async (t) => {
  const fake = await fakeRelease(t, { layouts: [['XTrace Desktop.app is at 208,151.'], []] });
  const result = await packageDmg(fake.options);
  assert.equal(result.name, 'XTrace-Desktop-0.1.4-macos-arm64.dmg');
  assert.equal(fake.calls.filter((call) => call.startsWith('pnpm tauri bundle')).length, 2);
  assert.equal(fake.keys.length, 2);
  const order = [
    'notarytool submit',
    'stapler staple',
    'stapler validate',
    'context:primary-signature',
  ];
  const onImage = fake.calls.filter((call) => call.includes('_aarch64.dmg'));
  const positions = order.map((step) => onImage.findIndex((call) => call.includes(step)));
  assert.ok(positions.every((position) => position >= 0));
  assert.deepEqual(
    [...positions].sort((a, b) => a - b),
    positions,
  );
  const output = join(fake.repo, 'artifacts/release', result.name);
  assert.equal(await readFile(output, 'utf8'), 'disk image bytes');
  assert.equal(
    await readFile(`${output}.sha256`, 'utf8'),
    `${result.sha256}  XTrace-Desktop-0.1.4-macos-arm64.dmg\n`,
  );
  // The key never appears on a command line, and its file is gone afterwards.
  assert.equal(
    fake.calls.some((call) => call.includes(env.APPLE_API_PRIVATE_KEY)),
    false,
  );
  assert.equal(existsSync(fake.keys[0]), false);
});

test(`packaging stops after ${LAYOUT_ATTEMPTS} misplaced layouts and before notarizing`, async (t) => {
  const wrong = ['XTrace Desktop.app is at 208,151.'];
  const fake = await fakeRelease(t, {
    layouts: Array.from({ length: LAYOUT_ATTEMPTS }, () => wrong),
  });
  await assert.rejects(packageDmg(fake.options), /layout was wrong in 3 builds/);
  assert.equal(
    fake.calls.some((call) => call.includes('notarytool')),
    false,
  );
});

test('a disk image Apple does not accept is never stapled or copied out', async (t) => {
  const fake = await fakeRelease(t, { layouts: [[]], notarization: 'Invalid' });
  await assert.rejects(packageDmg(fake.options), /did not accept the disk image \(Invalid\)/);
  assert.ok(fake.calls.some((call) => call.startsWith('xcrun notarytool log')));
  assert.equal(
    fake.calls.some((call) => call.includes('stapler staple')),
    false,
  );
});

const appCheckFailures = {
  'no hardened runtime': [
    (command, args) =>
      command === 'codesign' && args[0] === '-dvv' && args[1].endsWith('.app')
        ? { status: 0, stderr: signed.replace('flags=0x10000(runtime)', 'flags=0x0(none)') }
        : undefined,
    /without the hardened runtime/,
  ],
  'signed but not notarized': [
    (command, args) =>
      command === 'spctl' && args.includes('execute')
        ? { status: 0, stderr: 'X: accepted\nsource=Developer ID' }
        : undefined,
    /does not accept the app as notarized/,
  ],
  'no stapled ticket on the app': [
    (command, args) =>
      args[0] === 'stapler' && args[1] === 'validate' && args[2].endsWith('.app')
        ? { status: 65 }
        : undefined,
    /stapler failed/,
  ],
};

for (const [name, [tool, message]] of Object.entries(appCheckFailures)) {
  test(`an app with ${name} stops packaging before the disk image is notarized`, async (t) => {
    const fake = await fakeRelease(t, { layouts: [[]], tool });
    await assert.rejects(packageDmg(fake.options), message);
    assert.equal(
      fake.calls.some((call) => call.includes('notarytool submit')),
      false,
    );
  });
}

test('the app inside the final disk image is checked again', async (t) => {
  let mounted = false;
  const fake = await fakeRelease(t, {
    layouts: [[]],
    tool: (command, args) => {
      if (command === 'xcrun' && args[1] === 'staple') mounted = true;
      return mounted && command === 'spctl' && args.includes('execute')
        ? { status: 0, stderr: 'X: rejected' }
        : undefined;
    },
  });
  await assert.rejects(packageDmg(fake.options), /does not accept the app as notarized/);
  assert.equal(existsSync(join(fake.repo, 'artifacts/release')), false);
});

test('a misplaced icon in the final disk image is never copied out', async (t) => {
  const fake = await fakeRelease(t, {
    layouts: [[]],
    finalIcons: [iloc('Applications', 480, 170), iloc('XTrace Desktop.app', 208, 151)],
  });
  await assert.rejects(packageDmg(fake.options), /Final disk image layout: .*208,151/);
  assert.equal(existsSync(join(fake.repo, 'artifacts/release')), false);
});

test('a submission still in progress reports its status even though Apple has no log yet', async (t) => {
  const fake = await fakeRelease(t, {
    layouts: [[]],
    notarization: 'In Progress',
    tool: (command, args) => (args[1] === 'log' ? { status: 69, stderr: 'no log' } : undefined),
  });
  await assert.rejects(packageDmg(fake.options), /did not accept the disk image \(In Progress\)/);
});
