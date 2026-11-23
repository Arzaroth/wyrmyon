String formatBytes(int bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  final digits = unit == 0 || value >= 100 ? 0 : 1;
  return '${value.toStringAsFixed(digits)} ${units[unit]}';
}

String formatAge(DateTime then, DateTime now) {
  final age = now.difference(then);
  if (age.inMinutes < 1) return 'just now';
  if (age.inHours < 1) return '${age.inMinutes} min ago';
  if (age.inDays < 1) return '${age.inHours} h ago';
  if (age.inDays == 1) return 'yesterday';
  return '${age.inDays} days ago';
}

String formatDuration(Duration took) {
  final seconds = took.inSeconds;
  final minutes = seconds ~/ 60;
  final rest = (seconds % 60).toString().padLeft(2, '0');
  if (minutes < 60) return '$minutes:$rest';
  return '${minutes ~/ 60}:${(minutes % 60).toString().padLeft(2, '0')}:$rest';
}

String formatTook(Duration took) => took < const Duration(seconds: 1)
    ? 'in under a second'
    : 'in ${formatDuration(took)}';
