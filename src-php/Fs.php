<?php

declare(strict_types=1);

namespace Fairlead\Composer;

final class Fs
{
    /** A new, empty folder of our own inside $parent, as mkdtemp makes one. */
    public static function tempDir(string $parent): string
    {
        for ($try = 0; $try < 16; $try++) {
            $dir = $parent . '/.tmp-' . bin2hex(random_bytes(6));
            if (@mkdir($dir, 0700)) {
                return $dir;
            }
        }
        throw new LauncherError("can't create a folder in {$parent}");
    }

    public static function remove(string $path): void
    {
        if (is_link($path) || is_file($path)) {
            @unlink($path);
            return;
        }
        if (!is_dir($path)) {
            return;
        }
        foreach (scandir($path) ?: [] as $entry) {
            if ($entry !== '.' && $entry !== '..') {
                self::remove($path . '/' . $entry);
            }
        }
        @rmdir($path);
    }

    /** The first $name on the PATH, with Windows' executable extensions, or null. */
    public static function which(string $name, ?string $path = null, bool $windows = PHP_OS_FAMILY === 'Windows'): ?string
    {
        $path ??= (string) getenv('PATH');
        $exts = $windows ? array_merge([''], explode(';', strtolower((string) (getenv('PATHEXT') ?: '.exe;.bat;.cmd')))) : [''];
        foreach (explode(PATH_SEPARATOR, $path) as $dir) {
            foreach ($exts as $ext) {
                $candidate = rtrim($dir, '/\\') . DIRECTORY_SEPARATOR . $name . $ext;
                if ($dir !== '' && is_file($candidate) && ($windows || is_executable($candidate))) {
                    return $candidate;
                }
            }
        }
        return null;
    }
}
