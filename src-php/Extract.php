<?php

declare(strict_types=1);

namespace Fairlead\Composer;

final class Extract
{
    /**
     * Unpacks $archive into $into and answers the folder holding $binary: the
     * tarballs keep it in a folder named after the build, the zip at its top.
     */
    public static function run(string $archive, string $into, string $binary): string
    {
        if (!@mkdir($into, 0700)) {
            throw new LauncherError("can't create {$into}");
        }
        if (str_ends_with($archive, '.zip') && class_exists(\ZipArchive::class)) {
            self::unzip($archive, $into);
        } else {
            self::untar($archive, $into);
        }
        foreach ([$into, ...(glob($into . '/*', GLOB_ONLYDIR) ?: [])] as $dir) {
            if (is_file($dir . '/' . $binary)) {
                return $dir;
            }
        }
        throw new LauncherError(basename($archive) . " holds no {$binary}");
    }

    private static function unzip(string $archive, string $into): void
    {
        $zip = new \ZipArchive();
        if ($zip->open($archive) !== true || !$zip->extractTo($into)) {
            throw new LauncherError("can't unpack " . basename($archive));
        }
        $zip->close();
    }

    private static function untar(string $archive, string $into): void
    {
        $p = proc_open(
            [self::tar(), '-xf', $archive, '-C', $into],
            [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']],
            $pipes,
            null,
            null,
            ['bypass_shell' => true]
        );
        if (!is_resource($p)) {
            throw new LauncherError("can't run tar to unpack " . basename($archive));
        }
        fclose($pipes[0]);
        $out = stream_get_contents($pipes[1]) . stream_get_contents($pipes[2]);
        fclose($pipes[1]);
        fclose($pipes[2]);
        if (proc_close($p) !== 0) {
            throw new LauncherError("tar couldn't unpack " . basename($archive) . ': ' . trim($out));
        }
    }

    // On Windows, Git's GNU tar can come first on the PATH and reads `C:\...` as a remote
    // host, so the tar Windows ships is named by path.
    private static function tar(): string
    {
        if (PHP_OS_FAMILY === 'Windows') {
            $system = (getenv('SystemRoot') ?: 'C:\\Windows') . '\\System32\\tar.exe';
            if (is_file($system)) {
                return $system;
            }
        }
        return 'tar';
    }
}
