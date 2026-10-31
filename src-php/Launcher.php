<?php

declare(strict_types=1);

namespace Fairlead\Composer;

final class Launcher
{
    /** @param list<string> $argv */
    public static function main(array $argv, string $packageDir): never
    {
        try {
            $bin = self::binary(getenv(), $packageDir);
        } catch (\Throwable $e) {
            fwrite(STDERR, 'fairlead: ' . $e->getMessage() . "\n");
            exit(1);
        }
        Runner::run($bin, array_slice($argv, 1));
    }

    /** @param array<string, string> $env */
    public static function binary(array $env, string $packageDir): string
    {
        $version = Version::resolve($env['FAIRLEAD_VERSION'] ?? null, Version::installed());
        $libc = PHP_OS_FAMILY === 'Linux' ? Target::libc() : null;
        $triple = Target::triple(PHP_OS_FAMILY, php_uname('m'), $libc);
        if ($triple === null) {
            $libcName = $libc === null ? '' : ", {$libc['family']}";
            throw new LauncherError(
                'no release build runs on this platform (' . PHP_OS_FAMILY . ' ' . php_uname('m') . "{$libcName}).\n"
                . self::fromSource($version)
            );
        }
        $roots = Cache::roots($env, $packageDir);
        $cached = Cache::find($roots, $version, $triple);
        if ($cached !== null) {
            return $cached;
        }
        $root = Cache::writable($roots) ?? throw new LauncherError(
            'no writable cache folder for the binary; set FAIRLEAD_CACHE_DIR to one. Tried: '
            . implode(', ', array_column($roots, 'path'))
        );
        $attest = Runner::attestation($env['FAIRLEAD_VERIFY_ATTESTATION'] ?? '', Fs::which('gh'));
        $installer = new Installer(new HttpDownloader(self::caBundle()), null, $attest);
        return $installer->install($root, $version, $triple);
    }

    public static function fromSource(string $version): string
    {
        return "Build it from source with Rust, then run that binary instead:\n"
            . "  cargo install --locked --git https://github.com/mahkassem/fairlead --tag v{$version} fairlead";
    }

    // PHP on Windows has no CA store of its own; composer/ca-bundle finds the system's or ships one.
    private static function caBundle(): ?string
    {
        $bundle = 'Composer\\CaBundle\\CaBundle';
        return class_exists($bundle) ? $bundle::getSystemCaRootBundlePath() : null;
    }
}
