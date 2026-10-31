<?php

declare(strict_types=1);

namespace Fairlead\Composer;

/** The release build a platform runs, named by its Rust target triple. */
final class Target
{
    /** The oldest glibc the glibc builds run on (docs/src/install.md, Platforms). */
    public const GLIBC_MINIMUM = '2.35';

    /**
     * @param string $family PHP_OS_FAMILY
     * @param string $machine php_uname('m')
     * @param array{family: string, version?: ?string}|null $libc Linux only; null when it can't tell
     */
    public static function triple(string $family, string $machine, ?array $libc): ?string
    {
        $arch = match (strtolower($machine)) {
            'x86_64', 'amd64', 'x64' => 'x86_64',
            'aarch64', 'arm64' => 'aarch64',
            default => null,
        };
        if ($arch === null) {
            return null;
        }
        return match ($family) {
            // ARM64 Windows runs the x86_64 build under emulation; there is no ARM64 build.
            'Windows' => 'x86_64-pc-windows-msvc',
            'Darwin' => "{$arch}-apple-darwin",
            'Linux' => "{$arch}-unknown-linux-" . (self::glibcRuns($libc) ? 'gnu' : 'musl'),
            default => null,
        };
    }

    // The musl builds are static, so they also cover a C library that isn't
    // recognised and a glibc too old for the glibc build.
    private static function glibcRuns(?array $libc): bool
    {
        if ($libc === null || $libc['family'] !== 'glibc') {
            return false;
        }
        $version = $libc['version'] ?? null;
        return $version === null || version_compare($version, self::GLIBC_MINIMUM, '>=');
    }

    /**
     * What the system's C library is, from `ldd --version`, then the musl loader in /lib.
     * ldd goes first: a glibc system can have musl installed beside it, loader and all.
     *
     * @param (callable(): list<string>)|null $loaders
     * @param (callable(): ?string)|null $ldd
     * @return array{family: string, version?: ?string}|null
     */
    public static function libc(?callable $loaders = null, ?callable $ldd = null): ?array
    {
        $loaders ??= static fn (): array => glob('/lib/ld-musl-*') ?: [];
        $ldd ??= static fn (): ?string => self::lddVersion();
        $out = $ldd() ?? '';
        if (preg_match('/musl/i', $out)) {
            return ['family' => 'musl'];
        }
        if (preg_match('/glibc|gnu libc|gnu c library/i', $out)) {
            $first = strtok($out, "\n") ?: '';
            return ['family' => 'glibc', 'version' => preg_match('/(\d+\.\d+)\s*$/', $first, $m) ? $m[1] : null];
        }
        return $loaders() !== [] ? ['family' => 'musl'] : null;
    }

    public static function archive(string $triple): string
    {
        return "fairlead-{$triple}" . (self::windows($triple) ? '.zip' : '.tar.xz');
    }

    public static function binary(string $triple): string
    {
        return self::windows($triple) ? 'fairlead.exe' : 'fairlead';
    }

    private static function windows(string $triple): bool
    {
        return str_contains($triple, '-windows-');
    }

    // musl's ldd prints its banner on stderr and exits 1, so both streams count.
    private static function lddVersion(): ?string
    {
        $p = @proc_open(['ldd', '--version'], [0 => ['file', '/dev/null', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
        if (!is_resource($p)) {
            return null;
        }
        $out = stream_get_contents($pipes[1]) . stream_get_contents($pipes[2]);
        fclose($pipes[1]);
        fclose($pipes[2]);
        proc_close($p);
        return $out;
    }
}
