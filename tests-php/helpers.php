<?php

declare(strict_types=1);

use Fairlead\Composer\Downloader;
use Fairlead\Composer\Fs;
use Fairlead\Composer\NotFound;

final class Harness
{
    /** @var list<array{string, Closure}> */
    public static array $cases = [];
    /** @var list<string> */
    public static array $temps = [];

    public static function cleanup(): void
    {
        foreach (self::$temps as $dir) {
            Fs::remove($dir);
        }
        self::$temps = [];
    }
}

final class Skip extends \Exception
{
}

function test(string $name, Closure $case): void
{
    Harness::$cases[] = [$name, $case];
}

function check(bool $condition, string $message): void
{
    if (!$condition) {
        throw new \RuntimeException($message);
    }
}

function eq(mixed $expected, mixed $actual, string $what = ''): void
{
    check($expected === $actual, ($what === '' ? '' : "{$what}: ") . 'expected ' . var_export($expected, true) . ', got ' . var_export($actual, true));
}

/** The message of the $class $fn throws; fails when it throws nothing. */
function throws(string $class, Closure $fn): string
{
    try {
        $fn();
    } catch (\Throwable $e) {
        check($e instanceof $class, "expected {$class}, got " . get_class($e) . ': ' . $e->getMessage());
        return $e->getMessage();
    }
    throw new \RuntimeException("expected {$class}, nothing was thrown");
}

function contains(string $needle, string $haystack): void
{
    check(str_contains($haystack, $needle), "expected to find {$needle} in:\n{$haystack}");
}

function tmpdir(): string
{
    $dir = sys_get_temp_dir() . '/fairlead-php-test-' . bin2hex(random_bytes(6));
    mkdir($dir, 0700);
    Harness::$temps[] = $dir;
    return $dir;
}

function unix(): void
{
    if (DIRECTORY_SEPARATOR !== '/') {
        throw new Skip('Unix only');
    }
}

/** Serves files from a map of URL to body; any other URL is a 404. Records what was asked. */
final class FixtureDownloader implements Downloader
{
    /** @var list<string> */
    public array $asked = [];

    /** @param array<string, string|Closure> $bodies */
    public function __construct(private array $bodies)
    {
    }

    public function fetch(string $url, string $dest): void
    {
        $this->asked[] = $url;
        $body = $this->bodies[$url] ?? throw new NotFound('HTTP 404');
        file_put_contents($dest, $body instanceof Closure ? $body() : $body);
    }
}

/** A stub binary: prints its arguments one per line and a line of stdin, then exits with $code. */
function stub(int $code = 0, string $marker = 'stub'): string
{
    return "#!/bin/sh\necho {$marker}\nfor a in \"\$@\"; do printf '[%s]\\n' \"\$a\"; done\n"
        . "IFS= read -r line && echo \"stdin: \$line\"\nexit {$code}\n";
}

/** A release archive like the real ones: a .tar.xz with a folder named after the build, or a flat .zip. */
function release(string $triple, string $binary, string $body): string
{
    $dir = tmpdir();
    $name = \Fairlead\Composer\Target::archive($triple);
    if (str_ends_with($name, '.zip')) {
        $zip = new ZipArchive();
        $zip->open("{$dir}/{$name}", ZipArchive::CREATE);
        $zip->addFromString($binary, $body);
        $zip->addFromString('README.md', "readme\n");
        $zip->close();
    } else {
        $inner = "fairlead-{$triple}";
        mkdir("{$dir}/{$inner}");
        file_put_contents("{$dir}/{$inner}/{$binary}", $body);
        file_put_contents("{$dir}/{$inner}/README.md", "readme\n");
        exec('tar -cJf ' . escapeshellarg("{$dir}/{$name}") . ' -C ' . escapeshellarg($dir) . ' ' . escapeshellarg($inner), $out, $code);
        check($code === 0, 'tar -cJf failed: ' . implode("\n", $out));
    }
    return (string) file_get_contents("{$dir}/{$name}");
}

/** A downloader serving $triple's archive at $version and its checksum file. */
function serving(string $version, string $triple, string $archive, ?string $sum = null): FixtureDownloader
{
    $name = \Fairlead\Composer\Target::archive($triple);
    $url = \Fairlead\Composer\Installer::RELEASES . "/v{$version}/{$name}";
    return new FixtureDownloader([
        $url => $archive,
        "{$url}.sha256" => ($sum ?? hash('sha256', $archive)) . " *{$name}\n",
    ]);
}

function quiet(): Closure
{
    return static function (string $line): void {
    };
}
