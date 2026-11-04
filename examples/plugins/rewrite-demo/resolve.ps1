# Mohona runs this with the link as the last argument and reads one JSON object from the output.
param([string]$Link)
$real = $Link -replace '/view/', '/download/'
@{ url = $real; referrer = $Link } | ConvertTo-Json -Compress
