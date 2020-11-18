using System.Collections.Generic;

namespace NPitaya.Metrics
{
    internal class MetricSpec
    {
        internal readonly string Name;
        internal readonly string Help;
        internal readonly string[] Labels;

        internal MetricSpec(string name, string help, string[] labels)
        {
            Name = name;
            Help = help;
            Labels = labels;
        }
    }

    public enum HistogramBucketType
    {
        Exponential,
        Linear
    }

    public struct CustomHistogramConfig
    {
        public readonly HistogramBucketType Kind;
        public readonly double Start;
        public readonly double Inc;
        public readonly uint Count;

        public CustomHistogramConfig(HistogramBucketType kind, double start, double inc, uint count)
        {

            Kind = kind;
            Start = start;
            Inc = inc;
            Count = count;
        }
    }

    internal class HistogramSpec : MetricSpec
    {
        public readonly CustomHistogramConfig BucketConfig;

        public HistogramSpec(
            string name,
            string help,
            string[] labels,
            CustomHistogramConfig bucketConfig): base(name, help, labels)
        {
            BucketConfig = bucketConfig;
        }
    }

    public class CustomMetrics
    {
        internal readonly List<MetricSpec> Counters;
        internal readonly List<MetricSpec> Gauges;
        internal readonly List<HistogramSpec> Histograms;

        public CustomMetrics()
        {
            Counters = new List<MetricSpec>();
            Gauges = new List<MetricSpec>();
            Histograms = new List<HistogramSpec>();
        }

        public void AddCounter(string name, string help = null, string[] labels = null)
        {
            Counters.Add(new MetricSpec(name, help, labels));
        }

        public void AddGauge(string name, string help = null, string[] labels = null)
        {
            Gauges.Add(new MetricSpec(name, help, labels));
        }

        public void AddHistogram(string name, CustomHistogramConfig bucketConfig, string help = null, string[] labels = null)
        {
            Histograms.Add(new HistogramSpec(name, help, labels, bucketConfig));
        }
    }
}