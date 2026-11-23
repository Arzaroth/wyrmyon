import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:qr_flutter/qr_flutter.dart';

import '../app.dart';
import '../engine.dart';
import '../format.dart';
import '../transfer.dart';

class TransferPage extends StatefulWidget {
  const TransferPage({super.key, required this.job, this.code});

  final Job job;
  final String? code;

  static Route<void> route(Job job, {String? code}) => MaterialPageRoute(
    builder: (_) => TransferPage(job: job, code: code),
  );

  @override
  State<TransferPage> createState() => _TransferPageState();
}

class _TransferPageState extends State<TransferPage> {
  TransferController? _transfer;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_transfer != null) return;
    final services = Services.of(context);
    _transfer = TransferController(
      engine: services.engine,
      platform: services.platform,
      job: widget.job,
      code: widget.code,
    )..start();
  }

  @override
  void dispose() {
    _transfer?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final transfer = _transfer!;
    return ListenableBuilder(
      listenable: transfer,
      builder: (context, _) => PopScope(
        canPop: transfer.ended,
        onPopInvokedWithResult: (popped, _) {
          if (!popped) {
            transfer.cancel();
            Navigator.of(context).pop();
          }
        },
        child: Scaffold(
          appBar: AppBar(
            title: Text(transfer.sending ? 'Sending' : 'Receiving'),
          ),
          body: Padding(
            padding: const EdgeInsets.all(24),
            child: _Body(transfer),
          ),
        ),
      ),
    );
  }
}

class _Body extends StatelessWidget {
  const _Body(this.transfer);

  final TransferController transfer;

  @override
  Widget build(BuildContext context) {
    final children = <Widget>[
      if (transfer.notice case final notice?)
        Card.outlined(
          child: ListTile(
            leading: const Icon(Icons.campaign_outlined),
            title: Text(notice),
          ),
        ),
      ...switch (transfer.phase) {
        Phase.waiting => _waiting(context),
        Phase.offered => _offered(context),
        Phase.running => _running(context),
        Phase.done => _done(context),
        Phase.failed => _failed(context),
        Phase.cancelled => _cancelled(context),
      },
    ];
    return ListView(children: children);
  }

  List<Widget> _waiting(BuildContext context) {
    final code = transfer.shownCode;
    final text = Theme.of(context).textTheme;
    if (code == null) {
      return [
        const SizedBox(height: 48),
        const Center(child: CircularProgressIndicator()),
        const SizedBox(height: 16),
        Center(
          child: Text(
            transfer.stage == Stage.paired
                ? 'Connected to the other side'
                : 'Waiting for the other side',
          ),
        ),
        const SizedBox(height: 32),
        _cancel(context),
      ];
    }
    return [
      Center(child: Text('Your code', style: text.titleMedium)),
      const SizedBox(height: 8),
      Center(child: SelectableText(code, style: text.headlineMedium)),
      const SizedBox(height: 8),
      Center(
        child: TextButton.icon(
          onPressed: () => Clipboard.setData(ClipboardData(text: code)),
          icon: const Icon(Icons.copy),
          label: const Text('Copy'),
        ),
      ),
      const SizedBox(height: 16),
      const Center(child: Text('On your computer, run')),
      Center(
        child: SelectableText(
          'wyrm receive $code',
          style: const TextStyle(fontFamily: 'monospace'),
        ),
      ),
      const SizedBox(height: 16),
      Center(
        child: Container(
          color: Colors.white,
          padding: const EdgeInsets.all(12),
          child: QrImageView(
            data: 'wormhole-transfer:$code',
            size: 180,
            backgroundColor: Colors.white,
          ),
        ),
      ),
      const SizedBox(height: 24),
      _cancel(context),
    ];
  }

  List<Widget> _offered(BuildContext context) {
    final offer = transfer.offer!;
    final text = Theme.of(context).textTheme;
    return [
      ListTile(
        leading: const Icon(Icons.insert_drive_file_outlined, size: 40),
        title: Text(offer.name, style: text.titleLarge),
        subtitle: Text(formatBytes(offer.size.toInt())),
      ),
      const ListTile(
        leading: Icon(Icons.folder_outlined),
        title: Text('Saved to Download/wyrmyon'),
      ),
      const SizedBox(height: 24),
      FilledButton(onPressed: transfer.accept, child: const Text('Accept')),
      const SizedBox(height: 8),
      OutlinedButton(
        onPressed: () async {
          final navigator = Navigator.of(context);
          await transfer.decline();
          navigator.pop();
        },
        child: const Text('Decline'),
      ),
    ];
  }

  List<Widget> _running(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final hashing = transfer.stage == Stage.hashing;
    final total = transfer.total;
    final fraction = total > 0 ? transfer.bytes / total : null;
    final route = transfer.route;
    return [
      if (transfer.job case SendFile(:final file))
        Text(file.name, style: text.titleLarge),
      if (transfer.offer case final offer?)
        Text(offer.name, style: text.titleLarge),
      const SizedBox(height: 24),
      Text(
        hashing
            ? 'Preparing'
            : transfer.stage == Stage.awaitingAck
            ? 'Waiting for the other side to confirm'
            : fraction == null
            ? 'Connecting'
            : '${(fraction * 100).floor()}%',
        style: text.headlineMedium,
      ),
      const SizedBox(height: 8),
      LinearProgressIndicator(value: fraction),
      const SizedBox(height: 8),
      if (total > 0)
        Text('${formatBytes(transfer.bytes)} of ${formatBytes(total)}'),
      if (route != null && !hashing) ...[
        const SizedBox(height: 8),
        Text(route, style: text.bodySmall),
      ],
      const SizedBox(height: 32),
      _cancel(context),
    ];
  }

  List<Widget> _done(BuildContext context) {
    final platform = Services.of(context).platform;
    final text = Theme.of(context).textTheme;
    final offer = transfer.offer;
    final saved = transfer.savedUri;
    return [
      const Icon(Icons.check_circle, size: 64, color: Colors.green),
      const SizedBox(height: 8),
      Center(
        child: Text(
          transfer.sending ? 'Sent' : 'Received',
          style: text.headlineMedium,
        ),
      ),
      const SizedBox(height: 16),
      if (offer != null && offer.kind == OfferKind.text) ...[
        Card.outlined(
          child: Padding(
            padding: const EdgeInsets.all(16),
            child: SelectableText(offer.text),
          ),
        ),
        TextButton.icon(
          onPressed: () => Clipboard.setData(ClipboardData(text: offer.text)),
          icon: const Icon(Icons.copy),
          label: const Text('Copy'),
        ),
      ],
      if (transfer.job case SendFile(:final file))
        Center(child: Text('${file.name} · ${formatBytes(file.size)}')),
      if (offer != null && offer.kind == OfferKind.file)
        Center(
          child: Text(
            '${offer.name} · ${formatBytes(offer.size.toInt())} '
            'in Download/wyrmyon',
          ),
        ),
      const SizedBox(height: 24),
      if (saved != null) ...[
        FilledButton.icon(
          onPressed: () => platform.openDownload(saved),
          icon: const Icon(Icons.open_in_new),
          label: const Text('Open'),
        ),
        const SizedBox(height: 8),
        OutlinedButton.icon(
          onPressed: () => platform.shareDownload(saved),
          icon: const Icon(Icons.share),
          label: const Text('Share'),
        ),
        const SizedBox(height: 8),
      ],
      TextButton(
        onPressed: () => Navigator.of(context).pop(),
        child: const Text('Done'),
      ),
    ];
  }

  List<Widget> _failed(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return [
      Icon(Icons.error_outline, size: 64, color: scheme.error),
      const SizedBox(height: 16),
      SelectableText(transfer.error ?? 'The transfer failed'),
      const SizedBox(height: 24),
      FilledButton(
        onPressed: () => Navigator.of(context).pop(),
        child: const Text('Back'),
      ),
    ];
  }

  List<Widget> _cancelled(BuildContext context) => [
    const Center(child: Text('Cancelled')),
    const SizedBox(height: 24),
    FilledButton(
      onPressed: () => Navigator.of(context).pop(),
      child: const Text('Back'),
    ),
  ];

  Widget _cancel(BuildContext context) => OutlinedButton(
    onPressed: () {
      transfer.cancel();
      unawaited(Navigator.of(context).maybePop());
    },
    child: const Text('Cancel'),
  );
}
