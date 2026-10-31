<?php

declare(strict_types=1);

namespace Fairlead\Composer;

/** Where binaries are kept: `<root>/<version>/<triple>/fairlead`, one folder per build, never rewritten. */
final class Cache
{
    /**
     * The roots, most preferred first: FAIRLEAD_CACHE_DIR alone when set; else the package's
     * own folder, so a cached vendor/ carries the binary, the user cache, and the system temp.
     *
     * @param array<string, string> $env
     * @return list<array{path: string, shared: bool}> `shared` marks a root other users can write beside
     */
    public static function roots(array $env, string $packageDir, bool $windows = PHP_OS_FAMILY === 'Windows'): array
    {
        $explicit = $env['FAIRLEAD_CACHE_DIR'] ?? '';
        if ($explicit !== '') {
            return [['path' => $explicit, 'shared' => false]];
        }
        $roots = [['path' => $packageDir . '/.cache', 'shared' => false]];
        $user = $windows
            ? ($env['LOCALAPPDATA'] ?? '')
            : (($env['XDG_CACHE_HOME'] ?? '') ?: (($env['HOME'] ?? '') === '' ? '' : $env['HOME'] . '/.cache'));
        if ($user !== '') {
            $roots[] = ['path' => $user . '/fairlead', 'shared' => false];
        }
        $temp = rtrim(sys_get_temp_dir(), '/\\');
        if ($windows) {
            $roots[] = ['path' => $temp . '/fairlead', 'shared' => false];
        } elseif (self::uid() !== null) {
            $roots[] = ['path' => $temp . '/fairlead-' . self::uid(), 'shared' => true];
        }
        return $roots;
    }

    /** @param list<array{path: string, shared: bool}> $roots */
    public static function find(array $roots, string $version, string $triple): ?string
    {
        foreach ($roots as $root) {
            $bin = self::path($root['path'], $version, $triple);
            if (is_file($bin) && self::trusted($root)) {
                return $bin;
            }
        }
        return null;
    }

    /** @param list<array{path: string, shared: bool}> $roots */
    public static function writable(array $roots): ?string
    {
        foreach ($roots as $root) {
            $dir = $root['path'];
            if ((is_dir($dir) || @mkdir($dir, $root['shared'] ? 0700 : 0777, true)) && is_writable($dir) && self::trusted($root)) {
                return $dir;
            }
        }
        return null;
    }

    public static function folder(string $root, string $version, string $triple): string
    {
        return "{$root}/{$version}/{$triple}";
    }

    public static function path(string $root, string $version, string $triple): string
    {
        return self::folder($root, $version, $triple) . '/' . Target::binary($triple);
    }

    // In a temp folder anyone can create, a folder of this name is only ours to run
    // binaries from when we own it and nobody else can write to it.
    private static function trusted(array $root): bool
    {
        if (!$root['shared']) {
            return true;
        }
        clearstatcache(true, $root['path']);
        $owner = @fileowner($root['path']);
        $perms = @fileperms($root['path']);
        return $owner !== false && $owner === self::uid() && $perms !== false && ($perms & 0022) === 0;
    }

    // Without the posix extension there is no telling who owns a temp folder, so none is used.
    private static function uid(): ?int
    {
        return function_exists('posix_geteuid') ? posix_geteuid() : null;
    }
}
