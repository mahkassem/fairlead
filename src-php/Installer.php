<?php

declare(strict_types=1);

namespace Fairlead\Composer;

/** Downloads a release build, checks it, and moves it into the cache whole. */
final class Installer
{
    public const RELEASES = 'https://github.com/mahkassem/fairlead/releases/download';

    private \Closure $log;
    private ?\Closure $attest;

    /**
     * @param (callable(string): void)|null $log progress, to stderr by default
     * @param (callable(string): void)|null $attest throws when the archive's attestation doesn't verify
     */
    public function __construct(private Downloader $downloader, ?callable $log = null, ?callable $attest = null)
    {
        $this->log = \Closure::fromCallable($log ?? static fn (string $line) => fwrite(STDERR, "fairlead: {$line}\n"));
        $this->attest = $attest === null ? null : \Closure::fromCallable($attest);
    }

    /** The cached binary for this build, installed under $root first if it isn't there. */
    public function install(string $root, string $version, string $triple): string
    {
        $final = Cache::folder($root, $version, $triple);
        $bin = Cache::path($root, $version, $triple);
        if (is_file($bin)) {
            return $bin;
        }
        $parent = dirname($final);
        if (!is_dir($parent) && !@mkdir($parent, 0777, true) && !is_dir($parent)) {
            throw new LauncherError("can't create the cache folder {$parent}");
        }
        $work = Fs::tempDir($parent);
        try {
            $archive = $this->download($work, $version, $triple);
            if ($this->attest !== null) {
                ($this->attest)($archive);
            }
            $staging = Extract::run($archive, $work . '/x', Target::binary($triple));
            @chmod($staging . '/' . Target::binary($triple), 0755);
            return self::place($staging, $final, $bin);
        } finally {
            Fs::remove($work);
            @rmdir($parent); // only when empty: a failed first install leaves no version folder
        }
    }

    /** The archive, in $work, once its checksum matches the release's. */
    private function download(string $work, string $version, string $triple): string
    {
        $name = Target::archive($triple);
        $url = self::RELEASES . "/v{$version}/{$name}";
        $sum = "{$work}/{$name}.sha256";
        $this->get("{$url}.sha256", $sum, $version, $triple);
        $expected = self::expected((string) file_get_contents($sum), "{$url}.sha256", $version);
        ($this->log)("downloading {$url}");
        $file = "{$work}/{$name}";
        $this->get($url, $file, $version, $triple);
        $actual = hash_file('sha256', $file);
        if (!hash_equals($expected, (string) $actual)) {
            throw new LauncherError(
                "the download from {$url} doesn't match its checksum: expected {$expected}, got {$actual}. "
                . "Nothing was installed.\n" . Launcher::fromSource($version)
            );
        }
        return $file;
    }

    private function get(string $url, string $dest, string $version, string $triple): void
    {
        try {
            $this->downloader->fetch($url, $dest);
        } catch (NotFound) {
            // Packagist lists a tag as soon as it's pushed, before its release has built.
            throw new LauncherError(
                "no build for v{$version} yet on this platform ({$triple}): {$url} answered 404. "
                . "If v{$version} was tagged recently, its release may still be building; try again later.\n"
                . Launcher::fromSource($version)
            );
        } catch (\RuntimeException $e) {
            throw new LauncherError("couldn't download {$url}: {$e->getMessage()}\n" . Launcher::fromSource($version));
        }
    }

    /** The hex digest from a `<sha256> *<name>` line. */
    public static function expected(string $text, string $url, string $version): string
    {
        if (!preg_match('/^\s*([0-9a-fA-F]{64})\b/', $text, $m)) {
            throw new LauncherError("{$url} holds no sha256 checksum\n" . Launcher::fromSource($version));
        }
        return strtolower($m[1]);
    }

    /**
     * Renames a whole extraction into place. Another run that got there first wins and its
     * copy is used; a folder without the binary, which no run of this launcher leaves, is moved aside.
     */
    public static function place(string $staging, string $final, string $bin): string
    {
        if (@rename($staging, $final)) {
            return $bin;
        }
        clearstatcache();
        if (is_file($bin)) {
            return $bin;
        }
        $aside = $final . '.stale-' . bin2hex(random_bytes(4));
        if (@rename($final, $aside)) {
            Fs::remove($aside);
        }
        if (@rename($staging, $final)) {
            return $bin;
        }
        clearstatcache();
        if (is_file($bin)) {
            return $bin;
        }
        throw new LauncherError("couldn't move the binary into {$final}");
    }
}
