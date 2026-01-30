#!/usr/bin/env python3
"""
Generate HTML visualizations from memory test CSV files.

Usage:
    # Compare Go vs Rust (legacy mode)
    python generate_memtest_plots.py --go go_results.csv --rust rust_results.csv -o plots.html

    # Compare any two files with custom labels
    python generate_memtest_plots.py --compare "frequent-sync:file1.csv" "payment-history:file2.csv" -o plots.html

    # Single file
    python generate_memtest_plots.py --go go_results.csv -o go_plots.html
"""

import argparse
import csv
import json
from pathlib import Path


def read_csv(filepath: str) -> dict:
    """Read CSV and extract time (minutes), RSS (MB), and Heap (MB)."""
    times = []
    rss = []
    heap = []

    with open(filepath, 'r') as f:
        reader = csv.DictReader(f)
        for row in reader:
            elapsed_sec = float(row['elapsed_sec'])
            times.append(round(elapsed_sec / 60, 2))  # Convert to minutes

            rss_bytes = int(row['rss_bytes'])
            rss.append(round(rss_bytes / (1024 * 1024), 2))  # Convert to MB

            # Handle different column names for heap
            heap_col = 'heap_alloc_bytes' if 'heap_alloc_bytes' in row else 'heap_allocated_bytes'
            heap_bytes = int(row[heap_col])
            heap.append(round(heap_bytes / (1024 * 1024), 2))  # Convert to MB

    return {'times': times, 'rss': rss, 'heap': heap}


def generate_html_compare(datasets: list[tuple[str, dict]], title: str) -> str:
    """Generate HTML comparing multiple datasets side by side."""

    colors = ['#4a9eff', '#ff9e4a', '#9eff4a', '#ff4a9e']  # Blue, orange, green, pink

    charts_html = []
    charts_js = []
    chart_inits = []
    summary_html = '<div class="summary">'

    for i, (label, data) in enumerate(datasets):
        safe_label = label.replace('-', '_').replace(' ', '_')
        color = colors[i % len(colors)]

        charts_html.append(f'<div class="chart-container"><canvas id="{safe_label}Rss"></canvas></div>')
        charts_html.append(f'<div class="chart-container"><canvas id="{safe_label}Heap"></canvas></div>')

        charts_js.append(f"const {safe_label}T = {json.dumps(data['times'])};")
        charts_js.append(f"const {safe_label}R = {json.dumps(data['rss'])};")
        charts_js.append(f"const {safe_label}H = {json.dumps(data['heap'])};")

        chart_inits.append(f"""new Chart(document.getElementById('{safe_label}Rss'), {{ type: 'line', data: {{ labels: {safe_label}T, datasets: [{{ data: {safe_label}R, borderColor: '{color}', borderWidth: 1, pointRadius: 0 }}] }}, options: {{ ...opts, plugins: {{ ...opts.plugins, title: {{ display: true, text: '{label} - RSS (MB)', color: '#fff' }} }} }} }});""")
        chart_inits.append(f"""new Chart(document.getElementById('{safe_label}Heap'), {{ type: 'line', data: {{ labels: {safe_label}T, datasets: [{{ data: {safe_label}H, borderColor: '#4aff9e', borderWidth: 1, pointRadius: 0 }}] }}, options: {{ ...opts, plugins: {{ ...opts.plugins, title: {{ display: true, text: '{label} - Heap (MB)', color: '#fff' }} }} }} }});""")

        rss_start, rss_end = data['rss'][0], data['rss'][-1]
        heap_start, heap_end = data['heap'][0], data['heap'][-1]
        summary_html += f'''
        <div class="stat-box">
            <h3>{label}</h3>
            <p>RSS: {rss_start:.1f} MB → {rss_end:.1f} MB ({rss_end - rss_start:+.1f} MB)</p>
            <p>Heap: {heap_start:.2f} MB → {heap_end:.2f} MB ({heap_end - heap_start:+.2f} MB)</p>
            <p>Duration: {data['times'][-1]:.1f} min</p>
        </div>'''

    summary_html += '</div>'

    html = f'''<!DOCTYPE html>
<html>
<head>
    <title>Memory Test Results - {title}</title>
    <script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
    <style>
        body {{ font-family: sans-serif; padding: 20px; background: #1a1a1a; color: #fff; }}
        .chart-container {{ width: 48%; display: inline-block; margin: 1%; }}
        h1 {{ text-align: center; }}
        canvas {{ background: #2a2a2a; border-radius: 8px; }}
        .summary {{ display: flex; justify-content: center; gap: 40px; margin: 20px 0; flex-wrap: wrap; }}
        .stat-box {{ background: #2a2a2a; padding: 15px 25px; border-radius: 8px; }}
        .stat-box h3 {{ margin-top: 0; color: #4a9eff; }}
        .stat-box p {{ margin: 5px 0; font-family: monospace; }}
    </style>
</head>
<body>
    <h1>Memory Test Results - {title}</h1>
    {summary_html}
    {''.join(charts_html)}
    <script>
        {chr(10).join(charts_js)}

        const opts = {{ responsive: true, scales: {{ x: {{ title: {{ display: true, text: 'Minutes', color: '#aaa' }}, ticks: {{ color: '#aaa' }} }}, y: {{ title: {{ display: true, text: 'MB', color: '#aaa' }}, ticks: {{ color: '#aaa' }} }} }}, plugins: {{ legend: {{ display: false }} }} }};

        {chr(10).join(chart_inits)}
    </script>
</body>
</html>'''

    return html


def generate_html(go_data: dict | None, rust_data: dict | None, title: str) -> str:
    """Generate HTML with Chart.js visualizations (legacy Go vs Rust mode)."""
    datasets = []
    if go_data:
        datasets.append(('Go', go_data))
    if rust_data:
        datasets.append(('Rust', rust_data))
    return generate_html_compare(datasets, title)


def main():
    parser = argparse.ArgumentParser(description='Generate memory test HTML visualizations')
    parser.add_argument('--go', help='Path to Go CSV results file')
    parser.add_argument('--rust', help='Path to Rust CSV results file')
    parser.add_argument('--compare', nargs='+', metavar='LABEL:FILE',
                        help='Compare multiple files: "label1:file1.csv" "label2:file2.csv"')
    parser.add_argument('--output', '-o', required=True, help='Output HTML file path')
    parser.add_argument('--title', '-t', default='Memory Test', help='Title for the report')

    args = parser.parse_args()

    if args.compare:
        # New comparison mode
        datasets = []
        for item in args.compare:
            if ':' not in item:
                parser.error(f'Invalid format "{item}". Use "label:filepath"')
            label, filepath = item.split(':', 1)
            datasets.append((label, read_csv(filepath)))
        html = generate_html_compare(datasets, args.title)
    elif args.go or args.rust:
        # Legacy Go vs Rust mode
        go_data = read_csv(args.go) if args.go else None
        rust_data = read_csv(args.rust) if args.rust else None
        html = generate_html(go_data, rust_data, args.title)
    else:
        parser.error('Provide --go/--rust or --compare')

    output_path = Path(args.output)
    output_path.write_text(html)
    print(f'Generated: {output_path.absolute()}')


if __name__ == '__main__':
    main()
