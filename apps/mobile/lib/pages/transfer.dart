import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:qr_flutter/qr_flutter.dart';

import '../app.dart';
import '../engine.dart';
import '../format.dart';
import '../transfer.dart';
import '../ui.dart';
import 'send.dart';

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
        child: _screen(context, transfer),
      ),
    );
  }

  Widget _screen(BuildContext context, TransferController transfer) {
    final title = transfer.sending ? 'Sending' : 'Receiving';
    return switch (transfer.phase) {
      Phase.waiting => Screen(
        title: title,
        body: _Waiting(transfer),
        foot: [_cancel(context, transfer)],
      ),
      Phase.offered => _Offered(transfer),
      Phase.running => Screen(
        title: title,
        body: _Running(transfer),
        foot: [_cancel(context, transfer)],
      ),
      Phase.done => _Done(transfer),
      Phase.failed || Phase.cancelled => _Ended(transfer),
    };
  }

  Widget _cancel(BuildContext context, TransferController transfer) =>
      Secondary(
        'Cancel',
        onPressed: () {
          transfer.cancel();
          unawaited(Navigator.of(context).maybePop());
        },
      );
}

({String status, bool iroh, bool relay}) _route(String route) {
  final iroh = route.startsWith('iroh');
  final relay = route.contains('relay');
  return (
    status: relay ? 'Connected · via relay' : 'Connected · direct',
    iroh: iroh,
    relay: relay,
  );
}

Widget _transport(bool iroh) => iroh
    ? const Pill('iroh · wyrmyon peer', icon: Icons.bolt)
    : const Pill('classic transit', icon: Icons.swap_horiz);

class _Notice extends StatelessWidget {
  const _Notice(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    return Panel(
      child: Row(
        spacing: 12,
        children: [
          const Icon(Icons.campaign_outlined, color: Palette.cyan),
          Expanded(child: Text(text, style: const TextStyle(fontSize: 14))),
        ],
      ),
    );
  }
}

class _Waiting extends StatelessWidget {
  const _Waiting(this.transfer);

  final TransferController transfer;

  @override
  Widget build(BuildContext context) {
    final code = transfer.shownCode;
    final notice = transfer.notice;
    if (code == null) {
      return ListView(
        padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 16),
        children: [
          if (notice != null) ...[_Notice(notice), const SizedBox(height: 24)],
          const SizedBox(height: 24),
          const Center(
            child: Ring(
              value: null,
              child: Icon(Icons.wifi_tethering, size: 48, color: Palette.cyan),
            ),
          ),
          const SizedBox(height: 28),
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            spacing: 10,
            children: [
              Dot(
                color: transfer.stage == Stage.paired
                    ? Palette.ok
                    : Palette.dim,
              ),
              Flexible(
                child: Text(
                  transfer.stage == Stage.paired
                      ? 'Connected to the other side'
                      : 'Waiting for the other side',
                  style: const TextStyle(color: Palette.dim, fontSize: 15),
                ),
              ),
            ],
          ),
        ],
      );
    }
    return ListView(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 8),
      children: [
        if (notice != null) ...[_Notice(notice), const SizedBox(height: 16)],
        const Label('Your code'),
        const SizedBox(height: 10),
        Panel(
          child: Row(
            children: [
              Expanded(
                child: FittedBox(
                  fit: BoxFit.scaleDown,
                  child: SelectableText(
                    code,
                    style: const TextStyle(fontFamily: mono, fontSize: 26),
                  ),
                ),
              ),
              IconButton(
                tooltip: 'Copy',
                color: Palette.cyan,
                icon: const Icon(Icons.copy),
                onPressed: () => Clipboard.setData(ClipboardData(text: code)),
              ),
            ],
          ),
        ),
        const SizedBox(height: 20),
        Center(
          child: Container(
            padding: const EdgeInsets.all(14),
            decoration: BoxDecoration(
              color: Colors.white,
              borderRadius: BorderRadius.circular(20),
            ),
            child: QrImageView(
              data: 'wormhole-transfer:$code',
              size: 190,
              padding: EdgeInsets.zero,
              backgroundColor: Colors.white,
            ),
          ),
        ),
        const SizedBox(height: 20),
        Panel(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            spacing: 8,
            children: [
              const Text('On your computer, run', style: subStyle),
              Code('wyrm receive $code'),
            ],
          ),
        ),
      ],
    );
  }
}

class _Offered extends StatelessWidget {
  const _Offered(this.transfer);

  final TransferController transfer;

  @override
  Widget build(BuildContext context) {
    final offer = transfer.offer!;
    final folder = offer.kind == OfferKind.folder;
    final tree = folder || offer.kind == OfferKind.files;
    final size = formatBytes(offer.size.toInt());
    return Screen(
      title: 'Incoming',
      body: ListView(
        padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 8),
        children: [
          if (transfer.notice case final notice?) ...[
            _Notice(notice),
            const SizedBox(height: 16),
          ],
          const SizedBox(height: 12),
          Center(
            child: Tile(
              tree ? Icons.folder_copy_outlined : iconFor(offer.name),
              size: 88,
            ),
          ),
          const SizedBox(height: 16),
          Text(
            TransferController.describe(offer),
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 23, fontWeight: FontWeight.w500),
          ),
          const SizedBox(height: 6),
          Text(
            switch (offer.kind) {
              OfferKind.folder => 'folder · ${offer.count} files · $size',
              OfferKind.files => '${offer.count} files · $size',
              _ => 'file · $size',
            },
            textAlign: TextAlign.center,
            style: subStyle,
          ),
          const SizedBox(height: 14),
          Center(child: _transport(offer.iroh)),
          const SizedBox(height: 28),
          const Label('Save to'),
          const SizedBox(height: 10),
          Item(
            icon: Icons.folder_outlined,
            name: folder
                ? 'Download/wyrmyon/${offer.name}'
                : 'Download/wyrmyon',
            sub: 'Listed on Home once it arrives',
          ),
        ],
      ),
      foot: [
        Primary('Accept', onPressed: transfer.accept),
        Secondary(
          'Decline',
          onPressed: () async {
            final navigator = Navigator.of(context);
            await transfer.decline();
            navigator.pop();
          },
        ),
      ],
    );
  }
}

class _Running extends StatelessWidget {
  const _Running(this.transfer);

  final TransferController transfer;

  @override
  Widget build(BuildContext context) {
    final hashing = transfer.stage == Stage.hashing;
    final acking = transfer.stage == Stage.awaitingAck;
    final total = transfer.total;
    final fraction = total > 0 ? transfer.bytes / total : null;
    final route = transfer.route;
    final described = route == null ? null : _route(route);
    final speed = transfer.speed;
    final files = transfer.files;
    final name = switch (transfer.job) {
      SendFile(:final file) => file.name,
      _ => transfer.offer?.name,
    };
    return ListView(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 8),
      children: [
        const SizedBox(height: 8),
        Center(
          child: Ring(
            value: acking ? 1 : fraction,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  hashing
                      ? 'Preparing'
                      : acking
                      ? 'Confirming'
                      : fraction == null
                      ? 'Connecting'
                      : '${(fraction * 100).floor()}%',
                  style: TextStyle(
                    fontSize: fraction == null || hashing || acking ? 20 : 40,
                    fontWeight: FontWeight.w700,
                  ),
                ),
                if (total > 0)
                  Text(
                    '${formatBytes(transfer.bytes)} / ${formatBytes(total)}',
                    style: subStyle,
                  ),
              ],
            ),
          ),
        ),
        const SizedBox(height: 22),
        if (described != null && !hashing) ...[
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            spacing: 10,
            children: [
              Dot(color: described.relay ? Palette.cyan : Palette.ok),
              Flexible(
                child: Text(
                  described.status,
                  style: const TextStyle(color: Palette.dim, fontSize: 15),
                ),
              ),
            ],
          ),
          const SizedBox(height: 12),
          Center(child: _transport(described.iroh)),
          const SizedBox(height: 22),
        ],
        if (files.length > 1)
          ..._files(files, speed, fraction)
        else if (name != null)
          _Current(name: name, speed: speed, fraction: fraction),
      ],
    );
  }

  List<Widget> _files(
    List<({String path, bool done, double? progress})> files,
    double? speed,
    double? fraction,
  ) {
    final current = files.indexWhere((f) => !f.done);
    final shown = <Widget>[];
    for (final (index, file) in files.indexed.take(60)) {
      shown.add(
        Padding(
          padding: const EdgeInsets.only(bottom: 10),
          child: index == current
              ? _Current(
                  name: file.path,
                  speed: speed,
                  fraction: file.progress ?? fraction,
                )
              : Panel(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 16,
                    vertical: 13,
                  ),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          file.path,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontSize: 15,
                            color: file.done ? Palette.fg : Palette.dim,
                          ),
                        ),
                      ),
                      if (file.done)
                        const Icon(Icons.check, size: 18, color: Palette.ok),
                    ],
                  ),
                ),
        ),
      );
    }
    if (files.length > 60) {
      shown.add(Text('and ${files.length - 60} more', style: subStyle));
    }
    return shown;
  }
}

class _Current extends StatelessWidget {
  const _Current({
    required this.name,
    required this.speed,
    required this.fraction,
  });

  final String name;
  final double? speed;
  final double? fraction;

  @override
  Widget build(BuildContext context) {
    return Panel(
      child: Column(
        spacing: 10,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  name,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    fontSize: 15,
                    fontWeight: FontWeight.w500,
                  ),
                ),
              ),
              if (speed case final speed?)
                Text('${formatBytes(speed.round())}/s', style: subStyle),
            ],
          ),
          Bar(fraction),
        ],
      ),
    );
  }
}

class _Done extends StatelessWidget {
  const _Done(this.transfer);

  final TransferController transfer;

  @override
  Widget build(BuildContext context) {
    final platform = Services.of(context).platform;
    final offer = transfer.offer;
    final saved = transfer.savedUri;
    final text = offer?.kind == OfferKind.text ? offer!.text : null;
    final summary = switch (transfer.job) {
      SendFile(:final file) => '${file.name} · ${formatBytes(file.size)}',
      SendTree(:final entries, :final size) =>
        '${entries.length} files · ${formatBytes(size)}',
      SendText() => 'Text',
      Receive() => switch (offer?.kind) {
        OfferKind.folder =>
          '${offer!.name}/ · ${transfer.saved} files · '
              '${formatBytes(offer.size.toInt())}',
        OfferKind.files =>
          '${transfer.saved} files · ${formatBytes(offer!.size.toInt())}',
        OfferKind.file => '${offer!.name} · ${formatBytes(offer.size.toInt())}',
        _ => 'Text',
      },
    };
    final took = transfer.took;
    void done() => Navigator.of(context).pop();
    return Screen(
      body: Centered(
        children: [
          const Center(child: Check()),
          const SizedBox(height: 18),
          Text(
            transfer.sending ? 'Sent' : 'Received',
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 28, fontWeight: FontWeight.w500),
          ),
          const SizedBox(height: 8),
          Text(
            took == null ? summary : '$summary ${formatTook(took)}',
            textAlign: TextAlign.center,
            style: subStyle,
          ),
          if (transfer.verified) ...[
            const SizedBox(height: 6),
            const Row(
              mainAxisAlignment: MainAxisAlignment.center,
              spacing: 6,
              children: [
                Icon(Icons.verified_outlined, size: 16, color: Palette.ok),
                Flexible(child: Text('Verified end to end', style: subStyle)),
              ],
            ),
          ],
          if (text != null) ...[
            const SizedBox(height: 24),
            Panel(
              child: SelectableText(text, style: const TextStyle(fontSize: 16)),
            ),
          ],
        ],
      ),
      foot: [
        if (transfer.sending) ...[
          Primary('Done', onPressed: done),
          Secondary(
            'Send more to this PC',
            icon: Icons.add,
            onPressed: () async {
              final navigator = Navigator.of(context);
              final files = await platform.pickFiles();
              if (files.isEmpty) return;
              await navigator.pushReplacement(SendPage.route(files: files));
            },
          ),
          const Text(
            'Each code works once: the computer shows a new one.',
            textAlign: TextAlign.center,
            style: subStyle,
          ),
        ] else if (text != null) ...[
          Primary(
            'Copy',
            icon: Icons.copy,
            onPressed: () => Clipboard.setData(ClipboardData(text: text)),
          ),
          Secondary('Done', onPressed: done),
        ] else ...[
          if (saved != null) ...[
            Primary(
              'Open',
              icon: Icons.open_in_new,
              onPressed: () => platform.openDownload(saved),
            ),
            Secondary(
              'Share…',
              icon: Icons.share,
              onPressed: () => platform.shareDownload(saved),
            ),
          ] else if (transfer.savedUris.isNotEmpty)
            Primary(
              'Share all',
              icon: Icons.share,
              onPressed: () => platform.shareDownloads(transfer.savedUris),
            ),
          Wrap(
            alignment: WrapAlignment.center,
            children: [
              Ghost(
                'Show in folder',
                icon: Icons.folder_open_outlined,
                onPressed: platform.showDownloads,
              ),
              Ghost('Done', onPressed: done),
            ],
          ),
        ],
      ],
    );
  }
}

class _Ended extends StatelessWidget {
  const _Ended(this.transfer);

  final TransferController transfer;

  @override
  Widget build(BuildContext context) {
    final cancelled = transfer.phase == Phase.cancelled;
    return Screen(
      body: Centered(
        children: [
          const Center(child: Check(failed: true)),
          const SizedBox(height: 18),
          Text(
            cancelled ? 'Cancelled' : 'Failed',
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 28, fontWeight: FontWeight.w500),
          ),
          if (!cancelled) ...[
            const SizedBox(height: 16),
            Panel(
              child: SelectableText(
                transfer.error ?? 'The transfer failed',
                style: const TextStyle(fontSize: 15),
              ),
            ),
          ],
        ],
      ),
      foot: [Primary('Back', onPressed: () => Navigator.of(context).pop())],
    );
  }
}
