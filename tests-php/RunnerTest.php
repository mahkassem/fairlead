<?php

declare(strict_types=1);

use Fairlead\Composer\Cache;
use Fairlead\Composer\Target;

/** A cache root holding a stub as this machine's binary for $version. */
function stubbed(string $version, int $code): string
{
    unix();
    $triple = Target::triple(PHP_OS_FAMILY, php_uname('m'), PHP_OS_FAMILY === 'Linux' ? Target::libc() : null)
        ?? throw new Skip('no release build for this machine');
    $root = tmpdir();
    mkdir(Cache::folder($root, $version, $triple), 0777, true);
    file_put_contents(Cache::path($root, $version, $triple), stub($code));
    chmod(Cache::path($root, $version, $triple), 0755);
    return $root;
}

/** @param list<string> $command @return array{int, string, string} */
function runWith(array $command, array $env, string $stdin): array
{
    $p = proc_open($command, [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes, null, $env + getenv());
    fwrite($pipes[0], $stdin);
    fclose($pipes[0]);
    $out = stream_get_contents($pipes[1]);
    $err = stream_get_contents($pipes[2]);
    return [proc_close($p), $out, $err];
}

const ARGS = ['plan', 'two words', '', '--x=$HOME', '"quoted"', '*'];
const EXPECTED = "stub\n[plan]\n[two words]\n[]\n[--x=\$HOME]\n[\"quoted\"]\n[*]\nstdin: piped in\n";

test('bin/fairlead passes arguments, stdin, stdout and the exit code through', function (): void {
    $root = stubbed('0.14.0', 7);
    [$code, $out, $err] = runWith(
        [PHP_BINARY, __DIR__ . '/../bin/fairlead', ...ARGS],
        ['FAIRLEAD_VERSION' => '0.14.0', 'FAIRLEAD_CACHE_DIR' => $root],
        "piped in\n"
    );
    eq(7, $code, $err);
    eq(EXPECTED, $out);
    eq('', $err);
});

test('without pcntl_exec the binary runs as a child with the same streams and exit code', function (): void {
    $root = stubbed('0.14.0', 3);
    $script = 'require ' . var_export(__DIR__ . '/../src-php/load.php', true) . ';'
        . 'exit(Fairlead\Composer\Runner::spawn($argv[1], array_slice($argv, 2)));';
    $bin = glob("{$root}/0.14.0/*/fairlead")[0];
    [$code, $out, $err] = runWith([PHP_BINARY, '-r', $script, $bin, ...ARGS], [], "piped in\n");
    eq(3, $code, $err);
    eq(EXPECTED, $out);
});

test('bin/fairlead exits 1 with the reason when it cannot get a binary', function (): void {
    [$code, $out, $err] = runWith(
        [PHP_BINARY, __DIR__ . '/../bin/fairlead', '--version'],
        ['FAIRLEAD_VERSION' => 'not-a-version', 'FAIRLEAD_CACHE_DIR' => tmpdir()],
        ''
    );
    eq(1, $code);
    eq('', $out);
    contains("fairlead: FAIRLEAD_VERSION=not-a-version isn't a release version", $err);
});
