<?php

declare(strict_types=1);

use Fairlead\Composer\Cache;
use Fairlead\Composer\Installer;
use Fairlead\Composer\LauncherError;
use Fairlead\Composer\NotFound;
use Fairlead\Composer\Runner;

const LINUX = 'x86_64-unknown-linux-gnu';
const WINDOWS = 'x86_64-pc-windows-msvc';

function leftovers(string $root): array
{
    $found = [];
    foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::SELF_FIRST) as $path => $_) {
        if (preg_match('/\.tmp-|\.stale-/', $path)) {
            $found[] = $path;
        }
    }
    return $found;
}

test('a .tar.xz build is checked, unpacked and cached under its version and triple', function (): void {
    unix();
    $root = tmpdir();
    $downloader = serving('0.14.0', LINUX, release(LINUX, 'fairlead', stub()));
    $bin = (new Installer($downloader, quiet()))->install($root, '0.14.0', LINUX);
    eq("{$root}/0.14.0/" . LINUX . '/fairlead', $bin);
    check(is_executable($bin), 'the binary is executable');
    contains('echo stub', (string) file_get_contents($bin));
    eq([], leftovers($root), 'temporary folders');
    $sum = Installer::RELEASES . '/v0.14.0/fairlead-' . LINUX . '.tar.xz';
    eq(["{$sum}.sha256", $sum], $downloader->asked, 'the checksum is fetched before the archive');
});

test('a .zip build unpacks from the top of the archive', function (): void {
    if (!class_exists(ZipArchive::class)) {
        throw new Skip('needs the zip extension');
    }
    $root = tmpdir();
    $downloader = serving('0.14.0', WINDOWS, release(WINDOWS, 'fairlead.exe', 'MZ'));
    $bin = (new Installer($downloader, quiet()))->install($root, '0.14.0', WINDOWS);
    eq("{$root}/0.14.0/" . WINDOWS . '/fairlead.exe', $bin);
    eq('MZ', file_get_contents($bin));
});

test('an archive that does not match its checksum is never unpacked', function (): void {
    $root = tmpdir();
    $archive = release(LINUX, 'fairlead', stub());
    $downloader = serving('0.14.0', LINUX, $archive, str_repeat('ab', 32));
    $message = throws(LauncherError::class, fn () => (new Installer($downloader, quiet()))->install($root, '0.14.0', LINUX));
    contains("doesn't match its checksum", $message);
    contains(Installer::RELEASES . '/v0.14.0/fairlead-' . LINUX . '.tar.xz', $message);
    contains('expected ' . str_repeat('ab', 32) . ', got ' . hash('sha256', $archive), $message);
    contains('cargo install --locked --git https://github.com/mahkassem/fairlead --tag v0.14.0 fairlead', $message);
    eq([], array_values(array_diff(scandir($root), ['.', '..'])), 'nothing is left in the cache');
});

test('a checksum file holding no checksum stops the install', function (): void {
    $root = tmpdir();
    $name = 'fairlead-' . LINUX . '.tar.xz';
    $url = Installer::RELEASES . "/v0.14.0/{$name}";
    $downloader = new FixtureDownloader([$url => 'x', "{$url}.sha256" => "<html>not found</html>\n"]);
    $message = throws(LauncherError::class, fn () => (new Installer($downloader, quiet()))->install($root, '0.14.0', LINUX));
    contains("{$url}.sha256 holds no sha256 checksum", $message);
});

test('a release with no build yet says so, with the URL and the source build', function (): void {
    $root = tmpdir();
    $message = throws(LauncherError::class, fn () => (new Installer(new FixtureDownloader([]), quiet()))->install($root, '0.15.0', LINUX));
    contains('no build for v0.15.0 yet', $message);
    contains(Installer::RELEASES . '/v0.15.0/fairlead-' . LINUX . '.tar.xz.sha256 answered 404', $message);
    contains('--tag v0.15.0 fairlead', $message);
    eq([], array_values(array_diff(scandir($root), ['.', '..'])), 'no empty version folder is left');
});

test('a failed download names the URL and the reason', function (): void {
    $offline = new class implements \Fairlead\Composer\Downloader {
        public function fetch(string $url, string $dest): void
        {
            throw new RuntimeException('Could not resolve host: github.com');
        }
    };
    $message = throws(LauncherError::class, fn () => (new Installer($offline, quiet()))->install(tmpdir(), '0.14.0', LINUX));
    contains("couldn't download " . Installer::RELEASES . '/v0.14.0/fairlead-' . LINUX . '.tar.xz.sha256: Could not resolve host', $message);
    contains('cargo install', $message);
});

test('a cached binary is used without downloading', function (): void {
    $root = tmpdir();
    mkdir("{$root}/0.14.0/" . LINUX, 0777, true);
    file_put_contents("{$root}/0.14.0/" . LINUX . '/fairlead', 'cached');
    $downloader = new FixtureDownloader([]);
    eq("{$root}/0.14.0/" . LINUX . '/fairlead', (new Installer($downloader, quiet()))->install($root, '0.14.0', LINUX));
    eq([], $downloader->asked);
});

test('a run that loses the race to place the binary uses the winner\'s copy', function (): void {
    $root = tmpdir();
    $name = 'fairlead-' . LINUX . '.tar.xz';
    $url = Installer::RELEASES . "/v0.14.0/{$name}";
    $archive = release(LINUX, 'fairlead', 'ours');
    $downloader = new FixtureDownloader([
        "{$url}.sha256" => hash('sha256', $archive) . " *{$name}\n",
        // Another run finishes while this one downloads.
        $url => static function () use ($root, $archive): string {
            mkdir("{$root}/0.14.0/" . LINUX);
            file_put_contents("{$root}/0.14.0/" . LINUX . '/fairlead', 'theirs');
            return $archive;
        },
    ]);
    $bin = (new Installer($downloader, quiet()))->install($root, '0.14.0', LINUX);
    eq('theirs', file_get_contents($bin), 'the cached binary is never overwritten');
    eq([], leftovers($root), 'the losing extraction is removed');
});

test('a cache folder without its binary is moved aside, not written into', function (): void {
    $root = tmpdir();
    mkdir("{$root}/0.14.0/" . LINUX, 0777, true);
    file_put_contents("{$root}/0.14.0/" . LINUX . '/partial', 'x');
    $bin = (new Installer(serving('0.14.0', LINUX, release(LINUX, 'fairlead', 'fresh')), quiet()))->install($root, '0.14.0', LINUX);
    eq('fresh', file_get_contents($bin));
    check(!file_exists("{$root}/0.14.0/" . LINUX . '/partial'), 'the old folder is gone');
    eq([], leftovers($root));
});

test('first runs at once each get a whole binary', function (): void {
    unix();
    $root = tmpdir();
    $fixtures = tmpdir();
    file_put_contents("{$fixtures}/archive", release(LINUX, 'fairlead', stub(0, 'whole')));
    $child = "{$fixtures}/child.php";
    file_put_contents($child, '<?php
        require ' . var_export(__DIR__ . '/../src-php/load.php', true) . ';
        require ' . var_export(__DIR__ . '/helpers.php', true) . ';
        $archive = file_get_contents($argv[2]);
        $downloader = serving("0.14.0", ' . var_export(LINUX, true) . ', $archive);
        // Every run downloads before any extracts, so all of them reach the rename together.
        usleep(200000);
        echo (new Fairlead\Composer\Installer($downloader, quiet()))->install($argv[1], "0.14.0", ' . var_export(LINUX, true) . ');
    ');
    $runs = [];
    for ($i = 0; $i < 6; $i++) {
        $runs[] = proc_open([PHP_BINARY, $child, $root, "{$fixtures}/archive"], [1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes[$i]);
    }
    foreach ($runs as $i => $run) {
        $out = stream_get_contents($pipes[$i][1]) . stream_get_contents($pipes[$i][2]);
        eq(0, proc_close($run), "run {$i}: {$out}");
        eq("{$root}/0.14.0/" . LINUX . '/fairlead', $out, "run {$i}");
    }
    exec(escapeshellarg("{$root}/0.14.0/" . LINUX . '/fairlead') . ' < /dev/null', $lines, $code);
    eq(['whole'], $lines);
    eq([], leftovers($root), 'no extraction is left behind');
});

test('an asked-for attestation check fails closed', function (): void {
    $archive = release(LINUX, 'fairlead', stub());
    $install = fn (?Closure $attest) => (new Installer(serving('0.14.0', LINUX, $archive), quiet(), $attest))->install(tmpdir(), '0.14.0', LINUX);

    $message = throws(LauncherError::class, fn () => $install(Runner::attestation('1', null)));
    contains("gh isn't on the PATH", $message);

    $asked = [];
    $failing = static function (array $command) use (&$asked): int {
        $asked = $command;
        return 1;
    };
    $message = throws(LauncherError::class, fn () => $install(Runner::attestation('1', '/usr/bin/gh', $failing)));
    contains('gh attestation verify failed for fairlead-' . LINUX . '.tar.xz', $message);
    eq(['/usr/bin/gh', 'attestation', 'verify'], array_slice($asked, 0, 3));
    eq(['--repo', 'mahkassem/fairlead'], array_slice($asked, 4));
    check(str_ends_with($asked[3], '/fairlead-' . LINUX . '.tar.xz'), 'the archive is verified, before unpacking');

    $passing = static fn (array $command): int => 0;
    check(is_file($install(Runner::attestation('yes', '/usr/bin/gh', $passing))), 'a verified build installs');
    eq(null, Runner::attestation('', null));
    eq(null, Runner::attestation('0', null));
});
