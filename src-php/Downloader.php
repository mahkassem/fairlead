<?php

declare(strict_types=1);

namespace Fairlead\Composer;

interface Downloader
{
    /** Writes the body at $url to $dest; throws NotFound on a 404 and RuntimeException otherwise. */
    public function fetch(string $url, string $dest): void;
}

/** HTTPS only, redirects included, verified against $ca (a file or a directory) when given. */
final class HttpDownloader implements Downloader
{
    private const AGENT = 'fairlead-composer-launcher';

    public function __construct(private ?string $ca = null, private ?bool $curl = null)
    {
    }

    public function fetch(string $url, string $dest): void
    {
        if (!str_starts_with($url, 'https://')) {
            throw new \RuntimeException("refusing a URL that isn't HTTPS: {$url}");
        }
        ($this->curl ?? extension_loaded('curl')) ? $this->viaCurl($url, $dest) : $this->viaStreams($url, $dest);
    }

    private function viaCurl(string $url, string $dest): void
    {
        $out = fopen($dest, 'wb') ?: throw new \RuntimeException("can't write {$dest}");
        $ch = curl_init($url);
        $https = defined('CURLOPT_PROTOCOLS_STR')
            ? [CURLOPT_PROTOCOLS_STR => 'https', CURLOPT_REDIR_PROTOCOLS_STR => 'https']
            : [CURLOPT_PROTOCOLS => CURLPROTO_HTTPS, CURLOPT_REDIR_PROTOCOLS => CURLPROTO_HTTPS];
        $ca = $this->ca === null ? [] : [is_dir($this->ca) ? CURLOPT_CAPATH : CURLOPT_CAINFO => $this->ca];
        curl_setopt_array($ch, $https + $ca + [
            CURLOPT_FILE => $out,
            CURLOPT_FOLLOWLOCATION => true,
            CURLOPT_MAXREDIRS => 10,
            CURLOPT_SSL_VERIFYPEER => true,
            CURLOPT_SSL_VERIFYHOST => 2,
            CURLOPT_CONNECTTIMEOUT => 30,
            // A stalled transfer fails instead of hanging a git hook forever.
            CURLOPT_LOW_SPEED_LIMIT => 1,
            CURLOPT_LOW_SPEED_TIME => 60,
            CURLOPT_USERAGENT => self::AGENT,
        ]);
        $ok = curl_exec($ch);
        $status = (int) curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
        $error = curl_error($ch);
        curl_close($ch);
        fclose($out);
        if ($ok === false) {
            throw new \RuntimeException($error);
        }
        self::check($status);
    }

    // Streams follow redirects to any scheme, so each hop is followed here and held to HTTPS.
    private function viaStreams(string $url, string $dest): void
    {
        for ($hop = 0; $hop <= 10; $hop++) {
            if (!str_starts_with($url, 'https://')) {
                throw new \RuntimeException("redirected to a URL that isn't HTTPS: {$url}");
            }
            $in = @fopen($url, 'rb', false, $this->context());
            if ($in === false) {
                throw new \RuntimeException(error_get_last()['message'] ?? 'the connection failed');
            }
            [$status, $location] = self::head(stream_get_meta_data($in)['wrapper_data'] ?? []);
            if ($status >= 300 && $status < 400 && $location !== null) {
                fclose($in);
                $url = str_starts_with($location, '/') ? preg_replace('#^(https://[^/]+).*#', '$1', $url) . $location : $location;
                continue;
            }
            if ($status === 200) {
                $out = fopen($dest, 'wb') ?: throw new \RuntimeException("can't write {$dest}");
                $copied = stream_copy_to_stream($in, $out);
                fclose($out);
                fclose($in);
                if ($copied === false) {
                    throw new \RuntimeException('the transfer was cut off');
                }
                return;
            }
            fclose($in);
            self::check($status);
        }
        throw new \RuntimeException('too many redirects');
    }

    /** @return resource */
    private function context()
    {
        $http = ['follow_location' => 0, 'ignore_errors' => true, 'timeout' => 60, 'user_agent' => self::AGENT];
        $proxy = getenv('https_proxy') ?: getenv('HTTPS_PROXY');
        if ($proxy && ($p = parse_url($proxy)) && isset($p['host'])) {
            $http['proxy'] = "tcp://{$p['host']}:" . ($p['port'] ?? 80);
        }
        $ssl = ['verify_peer' => true, 'verify_peer_name' => true];
        if ($this->ca !== null) {
            $ssl[is_dir($this->ca) ? 'capath' : 'cafile'] = $this->ca;
        }
        return stream_context_create(['http' => $http, 'ssl' => $ssl]);
    }

    /** @param list<string> $headers  @return array{int, ?string} the last response's status and Location */
    private static function head(array $headers): array
    {
        $status = 0;
        $location = null;
        foreach ($headers as $line) {
            if (preg_match('#^HTTP/\S+\s+(\d{3})#', $line, $m)) {
                $status = (int) $m[1];
                $location = null;
            } elseif (preg_match('#^Location:\s*(\S+)#i', $line, $m)) {
                $location = $m[1];
            }
        }
        return [$status, $location];
    }

    private static function check(int $status): void
    {
        if ($status === 404) {
            throw new NotFound('HTTP 404');
        }
        if ($status !== 200) {
            throw new \RuntimeException("HTTP {$status}");
        }
    }
}
